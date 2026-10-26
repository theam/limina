//! MCP OAuth discovery, registration, PKCE and refresh use the official rmcp client.
//! Pending PKCE state and grants are encrypted in the database, so callbacks survive a restart.
use super::{Connections, Credential, net};
use limina_core::{LiminaError, Principal, Result, now};
use limina_store::{
    args,
    models::{McpAuthorizationFlow, McpConnection},
};
use oauth2::TokenResponse;
use rmcp::transport::auth::{
    AuthorizationManager, AuthorizationMetadata, CredentialStore, InMemoryCredentialStore,
    InMemoryStateStore, OAuthClientConfig, StateStore, StoredAuthorizationState, StoredCredentials,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use url::Url;

#[derive(Serialize, Deserialize)]
pub(super) struct Grant {
    metadata: AuthorizationMetadata,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    credentials: StoredCredentials,
}
#[derive(Serialize, Deserialize)]
struct Flow {
    metadata: AuthorizationMetadata,
    client_id: String,
    client_secret: Option<String>,
    state: StoredAuthorizationState,
}
pub struct ConnectOutcome {
    pub body: Value,
    pub state: Option<String>,
}
pub struct CallbackOutcome {
    pub return_to: String,
    pub id: String,
    pub error: Option<&'static str>,
}

pub fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}
pub fn flow_cookie(state: &str) -> String {
    format!("limina_mcp_{}", &hash(state)[..16])
}
fn auth_error() -> LiminaError {
    LiminaError::invariant("The provider could not authorize this connection. It must support MCP authorization metadata, S256 PKCE and dynamic client registration.").with_code("connection_authorization_failed")
}

impl Connections {
    pub async fn connect(
        &self,
        id: &str,
        principal: &Principal,
        redirect_uri: &str,
        return_to: &str,
        browser: &str,
    ) -> Result<ConnectOutcome> {
        let _guard = self.lock(id).await;
        let row = self.owned(id, principal).await?;
        let http = Arc::new(net::OutboundHttp::new()?);
        let server = net::server_url(&row.server_url)?;
        net::check_resolution(&server).await?;
        if row.auth_kind != "oauth" {
            let credential = row
                .secret_ciphertext
                .as_deref()
                .map(|s| self.open::<Credential>(id, "credentials", s))
                .transpose()?;
            if ["bearer", "header"].contains(&row.auth_kind.as_str())
                && credential.as_ref().and_then(|c| c.secret.as_ref()).is_none()
            {
                return Err(LiminaError::invariant("Add a token before connecting."));
            }
            let result =
                probe(&http, &row, credential.as_ref().and_then(|c| c.secret.as_deref())).await;
            if result.is_err() {
                self.status(
                    id,
                    "ERROR",
                    Some(
                        "The server did not accept the connection. Check its URL and credentials.",
                    ),
                )
                .await?;
                return Err(LiminaError::transport(
                    "The server did not accept the connection. Check its URL and credentials.",
                ));
            }
            self.connected(id, None, None, None).await?;
            return Ok(ConnectOutcome {
                body: json!({"connection":self.get(id,principal).await?,"authorization_url":null}),
                state: None,
            });
        }
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(server.clone(), http.clone())
                .await
                .map_err(|_| auth_error())?;
        // A resource may advertise a metadata URL in its WWW-Authenticate challenge.
        let challenge = http
            .client()
            .post(server)
            .header("Accept", "application/json, text/event-stream")
            .json(&initialize())
            .send()
            .await
            .ok()
            .and_then(|response| {
                response
                    .headers()
                    .get("www-authenticate")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned)
            });
        let resolution = match challenge {
            Some(challenge) => manager.resolve_metadata_from_challenge(Some(&challenge)).await,
            None => manager.resolve_metadata().await,
        }
        .map_err(|_| auth_error())?;
        if resolution.metadata.registration_endpoint.is_none() {
            self.status(
                id,
                "ERROR",
                Some("This provider does not offer dynamic client registration."),
            )
            .await?;
            return Err(auth_error());
        }
        let metadata = resolution.metadata;
        for endpoint in [&metadata.authorization_endpoint, &metadata.token_endpoint]
            .into_iter()
            .chain(metadata.registration_endpoint.as_ref())
        {
            net::server_url(endpoint)?;
        }
        manager.set_metadata(metadata.clone());
        let states = InMemoryStateStore::new();
        manager.set_state_store(states.clone());
        let scopes = manager.select_scopes(None, &[]);
        let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let config = manager
            .register_client("Limina", redirect_uri, &scope_refs)
            .await
            .map_err(|_| auth_error())?;
        manager.configure_client(config.clone()).map_err(|_| auth_error())?;
        let authorization_url =
            manager.get_authorization_url(&scope_refs).await.map_err(|_| auth_error())?;
        net::server_url(&authorization_url)?;
        let url = Url::parse(&authorization_url).map_err(|_| auth_error())?;
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(auth_error)?;
        let stored = states.load(&state).await.map_err(|_| auth_error())?.ok_or_else(auth_error)?;
        let payload = self.seal(
            id,
            "flow",
            &Flow {
                metadata,
                client_id: config.client_id,
                client_secret: config.client_secret,
                state: stored,
            },
        )?;
        let at = now();
        let return_to = if return_to.starts_with('/')
            && !return_to.starts_with("//")
            && !return_to.contains(['\\', '\r', '\n'])
        {
            return_to
        } else {
            "/connections"
        };
        let mut tx = self.service.db().begin_write().await?;
        tx.execute(
            "DELETE FROM mcp_authorization_flows WHERE connection_id=$1 OR expires_at<$2",
            args![id, at],
        )
        .await?;
        tx.execute("INSERT INTO mcp_authorization_flows (state_hash,connection_id,subject,browser_hash,redirect_uri,return_to,payload_ciphertext,created_at,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",args![hash(&state),id,&principal.subject,hash(browser),redirect_uri,return_to,&payload,at,at+chrono::Duration::minutes(10)]).await?;
        tx.commit().await?;
        self.status(id, "NEEDS_AUTHORIZATION", None).await?;
        Ok(ConnectOutcome {
            body: json!({"connection":self.get(id,principal).await?,"authorization_url":authorization_url}),
            state: Some(state),
        })
    }
    pub async fn callback(
        &self,
        state: &str,
        browser: Option<&str>,
        code: Option<&str>,
        issuer: Option<&str>,
        denied: bool,
    ) -> Result<CallbackOutcome> {
        let mut tx = self.service.db().begin().await?;
        let flow: Option<McpAuthorizationFlow> = tx
            .fetch_optional(
                "SELECT * FROM mcp_authorization_flows WHERE state_hash=$1",
                args![hash(state)],
            )
            .await?;
        tx.commit().await?;
        let Some(flow) = flow else {
            return Ok(CallbackOutcome {
                return_to: "/connections".into(),
                id: String::new(),
                error: Some("expired"),
            });
        };
        let _guard = self.lock(&flow.connection_id).await;
        let fail = |error| CallbackOutcome {
            return_to: flow.return_to.clone(),
            id: flow.connection_id.clone(),
            error: Some(error),
        };
        if browser.map(hash).as_deref() != Some(flow.browser_hash.as_str()) {
            return Ok(fail("browser"));
        }
        // Claim the flow once. A duplicate callback cannot exchange the same code twice.
        let mut tx = self.service.db().begin_write().await?;
        let removed = tx
            .execute("DELETE FROM mcp_authorization_flows WHERE state_hash=$1", args![hash(state)])
            .await?;
        tx.commit().await?;
        if removed == 0 || flow.expires_at < now() {
            return Ok(fail("expired"));
        }
        if denied {
            return Ok(fail("denied"));
        }
        let Some(code) = code else {
            return Ok(fail("failed"));
        };
        let principal = Principal::new(&flow.subject, "connection-owner");
        let row = self.owned(&flow.connection_id, &principal).await?;
        let payload: Flow = self.open(&row.id, "flow", &flow.payload_ciphertext)?;
        let mut manager = AuthorizationManager::new_with_oauth_http_client(
            &row.server_url,
            Arc::new(net::OutboundHttp::new()?),
        )
        .await
        .map_err(|_| auth_error())?;
        // Discovery also restores the resource identifier used in the token request.
        if manager.resolve_metadata().await.is_err() {
            return Ok(fail("failed"));
        }
        manager.set_metadata(payload.metadata.clone());
        let mut config = OAuthClientConfig::new(&payload.client_id, &flow.redirect_uri);
        config.client_secret = payload.client_secret.clone();
        config.scopes = payload.state.requested_scopes.clone();
        manager.configure_client(config).map_err(|_| auth_error())?;
        let states = InMemoryStateStore::new();
        states.save(state, payload.state).await.map_err(|_| auth_error())?;
        manager.set_state_store(states);
        let credentials = InMemoryCredentialStore::new();
        manager.set_credential_store(credentials.clone());
        if manager.exchange_code_for_token_with_issuer(code, state, issuer).await.is_err() {
            self.status(&row.id, "ERROR", Some("The provider refused the sign-in. Connect again."))
                .await?;
            return Ok(fail("failed"));
        }
        let stored = credentials.load().await.map_err(|_| auth_error())?.ok_or_else(auth_error)?;
        let grant = Grant {
            metadata: payload.metadata,
            client_id: payload.client_id,
            client_secret: payload.client_secret,
            redirect_uri: flow.redirect_uri.clone(),
            credentials: stored,
        };
        let token = grant.credentials.token_response.as_ref().ok_or_else(auth_error)?;
        let http = net::OutboundHttp::new()?;
        if probe(&http, &row, Some(token.access_token().secret())).await.is_err() {
            self.status(
                &row.id,
                "ERROR",
                Some("The provider signed in, but the MCP server refused access."),
            )
            .await?;
            return Ok(fail("failed"));
        }
        self.save_grant(&row.id, &grant).await?;
        Ok(CallbackOutcome { return_to: flow.return_to, id: row.id, error: None })
    }
    async fn connected(
        &self,
        id: &str,
        encrypted: Option<String>,
        scopes: Option<String>,
        expires: Option<limina_core::Timestamp>,
    ) -> Result<()> {
        let mut tx = self.service.db().begin_write().await?;
        tx.execute("UPDATE mcp_connections SET status='CONNECTED',last_error=NULL,secret_ciphertext=COALESCE($1,secret_ciphertext),scopes=$2,token_expires_at=$3,connected_at=COALESCE(connected_at,$4),updated_at=$4 WHERE id=$5",args![encrypted,scopes,expires,now(),id]).await?;
        tx.commit().await?;
        Ok(())
    }
    async fn save_grant(&self, id: &str, grant: &Grant) -> Result<()> {
        let encrypted = self.seal(
            id,
            "credentials",
            &Credential {
                secret: None,
                grant: Some(Grant {
                    metadata: grant.metadata.clone(),
                    client_id: grant.client_id.clone(),
                    client_secret: grant.client_secret.clone(),
                    redirect_uri: grant.redirect_uri.clone(),
                    credentials: grant.credentials.clone(),
                }),
            },
        )?;
        let token = grant.credentials.token_response.as_ref().ok_or_else(auth_error)?;
        let expiry = grant
            .credentials
            .token_received_at
            .zip(token.expires_in())
            .and_then(|(at, ttl)| i64::try_from(at.saturating_add(ttl.as_secs())).ok())
            .and_then(|at| chrono::DateTime::from_timestamp(at, 0));
        self.connected(
            id,
            Some(encrypted),
            Some(grant.credentials.granted_scopes.join(" ")),
            expiry,
        )
        .await
    }
    pub(super) async fn refresh(&self, row: &McpConnection, grant: &mut Grant) -> Result<String> {
        if let Some(token) = &grant.credentials.token_response {
            let needs_refresh = match (grant.credentials.token_received_at, token.expires_in()) {
                (Some(received), Some(ttl)) => {
                    received.saturating_add(ttl.as_secs())
                        <= (now().timestamp().max(0) as u64).saturating_add(60)
                }
                _ => false,
            };
            if !needs_refresh {
                return Ok(token.access_token().secret().clone());
            }
        }
        let store = InMemoryCredentialStore::new();
        store.save(grant.credentials.clone()).await.map_err(|_| auth_error())?;
        let mut manager = AuthorizationManager::new_with_oauth_http_client(
            &row.server_url,
            Arc::new(net::OutboundHttp::new()?),
        )
        .await
        .map_err(|_| auth_error())?;
        manager.resolve_metadata().await.map_err(|_| refresh_transport_error())?;
        manager.set_metadata(grant.metadata.clone());
        let mut config = OAuthClientConfig::new(&grant.client_id, &grant.redirect_uri);
        config.client_secret = grant.client_secret.clone();
        manager.configure_client(config).map_err(|_| auth_error())?;
        manager.set_credential_store(store.clone());
        let token = manager.get_access_token().await.map_err(|error| match error {
            rmcp::transport::auth::AuthError::TokenExpired
            | rmcp::transport::auth::AuthError::AuthorizationRequired
            | rmcp::transport::auth::AuthError::TokenRefreshRejected(_) => auth_error(),
            _ => refresh_transport_error(),
        })?;
        grant.credentials = store.load().await.map_err(|_| auth_error())?.ok_or_else(auth_error)?;
        self.save_grant(&row.id, grant).await?;
        Ok(token)
    }
}

fn refresh_transport_error() -> LiminaError {
    LiminaError::transport("The connection provider is temporarily unavailable. Limina will retry.")
        .with_code("connection_failed")
        .with("retryable", true)
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"limina","version":env!("CARGO_PKG_VERSION")}}})
}
async fn probe(http: &net::OutboundHttp, row: &McpConnection, secret: Option<&str>) -> Result<()> {
    let mut request = http
        .client()
        .post(&row.server_url)
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize());
    if let Some(secret) = secret {
        request = if row.auth_kind == "header" {
            request.header(row.header_name.as_deref().unwrap_or("x-api-key"), secret)
        } else {
            request.bearer_auth(secret)
        };
    }
    let response = request.send().await.map_err(|_| auth_error())?;
    if !response.status().is_success() {
        return Err(auth_error());
    }
    use futures::StreamExt as _;
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| auth_error())?;
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err(auth_error());
        }
        body.extend_from_slice(&chunk);
        let text = String::from_utf8_lossy(&body);
        let value = serde_json::from_slice::<Value>(&body).ok().or_else(|| {
            text.lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
                .find(|v| v["id"] == 1)
        });
        if let Some(value) = value {
            return if value["id"] == 1
                && value["result"]["protocolVersion"].is_string()
                && value["result"]["serverInfo"].is_object()
            {
                Ok(())
            } else {
                Err(auth_error())
            };
        }
    }
    Err(auth_error())
}
