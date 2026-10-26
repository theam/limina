//! Personal remote MCP connections. Credentials are encrypted and projects opt in explicitly.
pub mod net;
pub mod oauth;

use limina_core::{LiminaError, Principal, Result, iso, new_uuid, now};
use limina_domain::{
    collaboration::{CollaborationService, Role},
    service::ChallengeService,
};
use limina_engines::McpServer;
use limina_store::{args, models::McpConnection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Connections {
    service: ChallengeService,
    // Serializes refresh/connect/disconnect for a connection in this runtime.
    locks: Arc<Mutex<BTreeMap<String, Weak<Mutex<()>>>>>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewConnection {
    pub name: String,
    pub server_url: String,
    #[serde(default = "default_auth")]
    pub auth: String,
    pub header_name: Option<String>,
    pub secret: Option<String>,
}
fn default_auth() -> String {
    "oauth".into()
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionChanges {
    pub name: Option<String>,
    pub header_name: Option<String>,
    pub secret: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Credential {
    secret: Option<String>,
    grant: Option<oauth::Grant>,
}

impl Connections {
    pub fn new(service: ChallengeService) -> Self {
        Self { service, locks: Arc::default() }
    }
    async fn lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            match locks.get(id).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(Mutex::new(()));
                    locks.insert(id.into(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }
    fn cipher(&self) -> Result<&limina_store::SecretCipher> {
        self.service
            .secret_cipher()
            .ok_or_else(|| LiminaError::invariant("Connection encryption is unavailable."))
    }
    fn seal<T: Serialize>(&self, id: &str, name: &str, value: &T) -> Result<String> {
        Ok(self.cipher()?.encrypt(
            id,
            name,
            &serde_json::to_string(value).map_err(limina_core::internal)?,
        ))
    }
    fn open<T: serde::de::DeserializeOwned>(&self, id: &str, name: &str, value: &str) -> Result<T> {
        serde_json::from_str(&self.cipher()?.decrypt(id, name, value)?)
            .map_err(limina_core::internal)
    }
    async fn row(&self, id: &str) -> Result<McpConnection> {
        let mut tx = self.service.db().begin().await?;
        let row =
            tx.fetch_optional("SELECT * FROM mcp_connections WHERE id = $1", args![id]).await?;
        tx.commit().await?;
        row.ok_or_else(|| LiminaError::not_found("Connection not found."))
    }
    async fn owned(&self, id: &str, principal: &Principal) -> Result<McpConnection> {
        let row = self.row(id).await?;
        if row.owner_subject != principal.subject {
            return Err(LiminaError::not_found("Connection not found."));
        }
        Ok(row)
    }
    pub async fn list(&self, principal: &Principal) -> Result<Vec<Value>> {
        let mut tx = self.service.db().begin().await?;
        let rows: Vec<McpConnection> = tx
            .fetch_all(
                "SELECT * FROM mcp_connections WHERE owner_subject = $1 ORDER BY created_at",
                args![&principal.subject],
            )
            .await?;
        tx.commit().await?;
        let mut result = Vec::new();
        for row in rows {
            result.push(self.view(&row, principal).await?);
        }
        Ok(result)
    }
    pub async fn get(&self, id: &str, principal: &Principal) -> Result<Value> {
        self.view(&self.owned(id, principal).await?, principal).await
    }
    async fn view(&self, row: &McpConnection, principal: &Principal) -> Result<Value> {
        let mut tx = self.service.db().begin().await?;
        let projects: Vec<(String, String)> = tx.fetch_all("SELECT c.slug, c.name FROM project_mcp_connections p JOIN challenges c ON c.id = p.challenge_id WHERE p.connection_id = $1 ORDER BY c.name", args![&row.id]).await?;
        tx.commit().await?;
        let access = CollaborationService::new(self.service.db().clone())
            .visible_project_slugs(principal)
            .await?;
        let projects: Vec<Value> = projects
            .into_iter()
            .filter(|(slug, _)| access.as_ref().is_none_or(|slugs| slugs.contains(slug)))
            .map(|(slug, name)| json!({"slug":slug,"name":name}))
            .collect();
        Ok(
            json!({"id":row.id,"name":row.name,"server_url":row.server_url,"auth":row.auth_kind,"header_name":row.header_name,"status":row.status,"last_error":row.last_error,"credential_configured":row.secret_ciphertext.is_some(),"scopes":row.scopes.as_deref().map(|s|s.split_whitespace().collect::<Vec<_>>()).unwrap_or_default(),"expires_at":row.token_expires_at.as_ref().map(iso),"connected_at":row.connected_at.as_ref().map(iso),"created_at":iso(&row.created_at),"updated_at":iso(&row.updated_at),"projects":projects}),
        )
    }
    pub async fn create(
        &self,
        input: NewConnection,
        principal: &Principal,
        command_id: &str,
    ) -> Result<Value> {
        let name = connection_name(&input.name)?;
        let url = net::server_url(&input.server_url)?;
        net::check_resolution(&url).await?;
        if !["oauth", "none", "bearer", "header"].contains(&input.auth.as_str()) {
            return Err(LiminaError::invariant("Choose sign-in, no sign-in, a token or a header."));
        }
        let header = header_name(&input.auth, input.header_name.as_deref())?;
        let secret = validate_secret(&input.auth, input.secret.as_deref())?;
        let id = new_uuid();
        let encrypted = secret
            .map(|secret| {
                self.seal(&id, "credentials", &Credential { secret: Some(secret), grant: None })
            })
            .transpose()?;
        let command_id = super::operations::scoped_command_id(principal, command_id);
        let stored_id: String = limina_domain::command!(
            self.service.db(),
            &command_id,
            "connection.create",
            &principal.actor(),
            |tx| {
                tx.execute("INSERT INTO mcp_connections (id,owner_subject,owner_name,name,server_url,auth_kind,header_name,status,secret_ciphertext,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,'NOT_CONNECTED',$8,$9,$9)",args![&id,&principal.subject,&principal.display_name,&name,url.as_str(),&input.auth,header.as_deref(),encrypted.as_deref(),now()]).await?;
                Ok(id.clone())
            }
        )?;
        self.get(&stored_id, principal).await
    }

    pub async fn update(
        &self,
        id: &str,
        input: ConnectionChanges,
        principal: &Principal,
    ) -> Result<Value> {
        let _guard = self.lock(id).await;
        let row = self.owned(id, principal).await?;
        let name = connection_name(input.name.as_deref().unwrap_or(&row.name))?;
        let header = header_name(
            &row.auth_kind,
            input.header_name.as_deref().or(row.header_name.as_deref()),
        )?;
        let encrypted = match input.secret {
            Some(secret) => {
                let secret = validate_secret(&row.auth_kind, Some(&secret))?;
                Some(self.seal(id, "credentials", &Credential { secret, grant: None })?)
            }
            None => row.secret_ciphertext.clone(),
        };
        let changed = encrypted != row.secret_ciphertext || header != row.header_name;
        let mut tx = self.service.db().begin_write().await?;
        tx.execute("UPDATE mcp_connections SET name=$1,header_name=$2,secret_ciphertext=$3,status=$4,last_error=NULL,updated_at=$5 WHERE id=$6",args![&name,header.as_deref(),encrypted.as_deref(),if changed {"NOT_CONNECTED"} else {&row.status},now(),id]).await?;
        tx.commit().await?;
        self.get(id, principal).await
    }
    pub async fn remove(&self, id: &str, principal: &Principal, delete: bool) -> Result<Value> {
        let _guard = self.lock(id).await;
        let row = self.owned(id, principal).await?;
        let view = self.view(&row, principal).await?;
        let mut tx = self.service.db().begin_write().await?;
        tx.execute("DELETE FROM project_mcp_connections WHERE connection_id=$1", args![id]).await?;
        tx.execute("DELETE FROM mcp_authorization_flows WHERE connection_id=$1", args![id]).await?;
        if delete {
            tx.execute("DELETE FROM mcp_connections WHERE id=$1", args![id]).await?;
        } else {
            tx.execute("UPDATE mcp_connections SET status='NOT_CONNECTED',secret_ciphertext=NULL,scopes=NULL,token_expires_at=NULL,last_error=NULL,updated_at=$1 WHERE id=$2",args![now(),id]).await?;
        }
        tx.commit().await?;
        if delete { Ok(view) } else { self.get(id, principal).await }
    }
    pub async fn for_project(&self, slug: &str, principal: &Principal) -> Result<Value> {
        let collaboration = CollaborationService::new(self.service.db().clone());
        let role = collaboration.require_role(slug, principal, Role::Viewer).await?;
        let mut tx = self.service.db().begin().await?;
        type ProjectRow = (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            limina_core::Timestamp,
            Option<String>,
            String,
            String,
        );
        let rows:Vec<ProjectRow>=tx.fetch_all("SELECT m.id,m.name,m.server_url,p.server_key,m.owner_subject,m.owner_name,m.status,p.created_at,pm.role,p.granted_as,p.enabled_by FROM project_mcp_connections p JOIN mcp_connections m ON m.id=p.connection_id JOIN challenges c ON c.id=p.challenge_id LEFT JOIN project_members pm ON pm.challenge_id=c.id AND pm.subject=m.owner_subject WHERE c.slug=$1 ORDER BY m.name",args![slug]).await?;
        tx.commit().await?;
        let mut enabled = Vec::new();
        let mut ids = Vec::new();
        for (id, name, url, key, owner, owner_name, status, at, membership, grant, enabled_by) in
            rows
        {
            let owned = owner == principal.subject;
            let owner_left = membership.is_none() && grant != "LOCAL";
            let reason = if owner_left {
                Some("owner_left")
            } else if status != "CONNECTED" {
                Some("not_connected")
            } else {
                None
            };
            ids.push(id.clone());
            enabled.push(json!({"connection_id":id,"name":name,"server_url":url,"server_key":key,"owner":owner_name,"owned_by_you":owned,"status":status,"active":reason.is_none(),"inactive_reason":reason,"enabled_by":enabled_by,"enabled_at":iso(&at)}));
        }
        let mut available = Vec::new();
        if role == Role::Owner {
            for item in self.list(principal).await? {
                if !ids.iter().any(|id| item["id"] == *id) {
                    available.push(json!({"connection_id":item["id"],"name":item["name"],"server_url":item["server_url"],"status":item["status"]}));
                }
            }
        }
        Ok(json!({"connections":enabled,"available":available,"can_enable":role==Role::Owner}))
    }
    pub async fn enable(&self, slug: &str, id: &str, principal: &Principal) -> Result<Value> {
        let collaboration = CollaborationService::new(self.service.db().clone());
        collaboration.require_role(slug, principal, Role::Owner).await?;
        let _guard = self.lock(id).await;
        let row = self.owned(id, principal).await?;
        if row.status != "CONNECTED" {
            return Err(LiminaError::invariant(
                "Connect this server before enabling it in a project.",
            ));
        }
        let mut tx = self.service.db().begin_write().await?;
        let project: String =
            tx.scalar("SELECT id FROM challenges WHERE slug=$1", args![slug]).await?;
        // Stable server names survive display-name changes and cannot collide.
        let key = format!("mcp_{}", id.replace('-', ""));
        tx.execute("INSERT INTO project_mcp_connections (challenge_id,connection_id,server_key,granted_as,enabled_by,created_at) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (challenge_id,connection_id) DO NOTHING",args![&project,id,&key,if principal.acts_on_every_project(){"LOCAL"}else{"MEMBER"},&principal.actor(),now()]).await?;
        tx.commit().await?;
        let view = self.for_project(slug, principal).await?;
        Ok(view["connections"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["connection_id"] == id))
            .cloned()
            .unwrap_or(Value::Null))
    }
    pub async fn disable(&self, slug: &str, id: &str, principal: &Principal) -> Result<Value> {
        CollaborationService::new(self.service.db().clone())
            .require_role(slug, principal, Role::Viewer)
            .await?;
        self.owned(id, principal).await?;
        let mut tx = self.service.db().begin_write().await?;
        tx.execute("DELETE FROM project_mcp_connections WHERE connection_id=$1 AND challenge_id=(SELECT id FROM challenges WHERE slug=$2)",args![id,slug]).await?;
        tx.commit().await?;
        Ok(json!({"connection_id":id}))
    }
    pub async fn resolve(&self, slug: &str) -> Result<Vec<McpServer>> {
        let mut tx = self.service.db().begin().await?;
        let rows:Vec<(String,String)>=tx.fetch_all("SELECT p.connection_id,p.server_key FROM project_mcp_connections p JOIN challenges c ON c.id=p.challenge_id JOIN mcp_connections m ON m.id=p.connection_id LEFT JOIN project_members pm ON pm.challenge_id=c.id AND pm.subject=m.owner_subject WHERE c.slug=$1 AND (pm.subject IS NOT NULL OR p.granted_as='LOCAL') ORDER BY p.server_key",args![slug]).await?;
        tx.commit().await?;
        let mut servers = Vec::new();
        for (id, key) in rows {
            let _guard = self.lock(&id).await;
            let row = self.row(&id).await?;
            if row.status != "CONNECTED" {
                continue;
            }
            let url = net::server_url(&row.server_url)?;
            net::check_resolution(&url).await?;
            let mut server = McpServer::new(key, url.as_str());
            if let Some(encrypted) = &row.secret_ciphertext {
                let mut credential: Credential = self.open(&id, "credentials", encrypted)?;
                if let Some(grant) = &mut credential.grant {
                    match self.refresh(&row, grant).await {
                        Ok(token) => server.bearer_token = Some(token),
                        Err(error) => {
                            if error.code() != "connection_failed" {
                                self.status(
                                    &id,
                                    "NEEDS_AUTHORIZATION",
                                    Some("Authorization expired. Reconnect this server."),
                                )
                                .await?;
                            }
                            return Err(error);
                        }
                    }
                } else if row.auth_kind == "bearer" {
                    server.bearer_token = credential.secret;
                } else if row.auth_kind == "header"
                    && let (Some(header), Some(secret)) = (&row.header_name, credential.secret)
                {
                    server.headers.insert(header.clone(), secret);
                }
            }
            servers.push(server);
        }
        Ok(servers)
    }
    async fn status(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        let mut tx = self.service.db().begin_write().await?;
        tx.execute(
            "UPDATE mcp_connections SET status=$1,last_error=$2,updated_at=$3 WHERE id=$4",
            args![status, error, now(), id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

fn connection_name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(LiminaError::invariant("Enter a name of 1 to 120 characters."));
    }
    Ok(name.into())
}
fn header_name(auth: &str, value: Option<&str>) -> Result<Option<String>> {
    if auth != "header" {
        if value.is_some() {
            return Err(LiminaError::invariant("Only header connections accept a header name."));
        }
        return Ok(None);
    }
    let value = value.unwrap_or_default().trim();
    let parsed = axum::http::HeaderName::from_bytes(value.as_bytes())
        .map_err(|_| LiminaError::invariant("Enter a valid header name."))?;
    if value.len() > 120
        || [
            "host",
            "cookie",
            "connection",
            "content-length",
            "transfer-encoding",
            "proxy-authorization",
            "authorization",
        ]
        .contains(&parsed.as_str())
    {
        return Err(LiminaError::invariant("Use a dedicated API key header."));
    }
    Ok(Some(parsed.to_string()))
}
fn validate_secret(auth: &str, value: Option<&str>) -> Result<Option<String>> {
    if ["bearer", "header"].contains(&auth) {
        let value = value.unwrap_or_default().trim();
        if value.is_empty()
            || value.len() > 16384
            || axum::http::HeaderValue::from_str(value).is_err()
        {
            return Err(LiminaError::invariant("Enter a valid token or header value."));
        }
        Ok(Some(value.into()))
    } else if value.is_some() {
        Err(LiminaError::invariant("This connection does not accept a pasted secret."))
    } else {
        Ok(None)
    }
}
