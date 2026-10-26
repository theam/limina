//! The hidden `_agent` protocol. Managed agents call these commands during a turn; each prints
//! one compact JSON line (`json.dumps(value, ensure_ascii=False)`), whatever `--json` says.

use limina_core::{LiminaError, Result};
use serde_json::Value;

use super::{Context, Outcome, command_id, database_url};
use crate::cli::{
    AgentCommand, AgentExperimentCommand, AgentFindingCommand, AgentHypothesisCommand,
};
use crate::client::protocol::{
    ExperimentBody, ExperimentClaimBody, ExperimentCompletionBody, FindingBody, HypothesisBody,
    HypothesisDecisionBody, ObservationBody,
};
use crate::client::{AgentClient, HttpRuntimeClient, LocalRuntimeClient};
use crate::output;

/// The project of the current managed turn (`LIMINA_PROJECT`, or the older `LIMINA_CHALLENGE`).
fn agent_project() -> Result<String> {
    ["LIMINA_PROJECT", "LIMINA_CHALLENGE"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .ok_or_else(|| {
            LiminaError::invariant("The internal agent protocol requires LIMINA_PROJECT.")
                .with("suggestion", "Run this command from a Limina-managed project turn.")
        })
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The internal API when the runtime provided a capability, otherwise the local database.
async fn agent_client(context: &Context) -> Result<AgentClient> {
    if let (Some(url), Some(token)) =
        (non_empty_env("LIMINA_INTERNAL_URL"), non_empty_env("LIMINA_INTERNAL_TOKEN"))
    {
        let lane = non_empty_env("LIMINA_AGENT_LANE");
        return Ok(AgentClient::Http(HttpRuntimeClient::new(&url, Some(&token), lane.as_deref())));
    }
    let local = LocalRuntimeClient::connect(&database_url(&context.database)).await?;
    Ok(AgentClient::Local(local))
}

pub async fn agent(context: &Context, command: AgentCommand) -> Outcome {
    let slug = agent_project()?;
    let client = agent_client(context).await?;
    let result = run(context, &client, &slug, command).await;
    client.close().await;
    output::line(&crate::pyjson::dumps(&result?));
    Ok(())
}

async fn run(
    context: &Context,
    client: &AgentClient,
    slug: &str,
    command: AgentCommand,
) -> Result<Value> {
    let actor = context.actor.as_str();
    let id = command_id();
    match command {
        AgentCommand::Status => client.status(slug).await,
        AgentCommand::List(args) => client.artifacts(slug, args.kind.as_deref()).await,
        AgentCommand::Show(args) => client.artifact(slug, &args.artifact_id.to_uppercase()).await,
        AgentCommand::Hypothesis(AgentHypothesisCommand::Add(args)) => {
            let body = HypothesisBody {
                title: args.title,
                statement: args.statement,
                mechanism: args.mechanism,
                generalization: args.generalization,
                shortcut_risks: args.shortcut_risks,
                test_plan: args.test_plan,
            };
            client.create_hypothesis(slug, body, actor, &id).await
        }
        AgentCommand::Hypothesis(AgentHypothesisCommand::Decide(args)) => {
            let body = HypothesisDecisionBody {
                status: args.status,
                conclusion: args.conclusion,
                expected_version: args.expect,
            };
            client.decide_hypothesis(slug, &args.artifact_id.to_uppercase(), body, actor, &id).await
        }
        AgentCommand::Experiment(AgentExperimentCommand::Add(args)) => {
            let body = ExperimentBody {
                hypothesis_id: args.hypothesis_id.to_uppercase(),
                title: args.title,
                objective: args.objective,
                procedure: args.procedure,
                success_criteria: args.success,
                guardrails: args.guardrails,
            };
            client.create_experiment(slug, body, actor, &id).await
        }
        AgentCommand::Experiment(AgentExperimentCommand::Claim(args)) => {
            let body = ExperimentClaimBody { ttl_seconds: args.ttl };
            client.claim_experiment(slug, &args.artifact_id.to_uppercase(), body, actor, &id).await
        }
        AgentCommand::Experiment(AgentExperimentCommand::Observe(args)) => {
            let body = ObservationBody { body: args.observation, evidence_ref: args.evidence };
            client
                .observe_experiment(slug, &args.artifact_id.to_uppercase(), body, actor, &id)
                .await
        }
        AgentCommand::Experiment(AgentExperimentCommand::Complete(args)) => {
            let body = ExperimentCompletionBody {
                results: args.results,
                analysis: args.analysis,
                decision: args.decision,
                expected_version: args.expect,
            };
            client
                .complete_experiment(slug, &args.artifact_id.to_uppercase(), body, actor, &id)
                .await
        }
        AgentCommand::Finding(AgentFindingCommand::Publish(args)) => {
            let body = FindingBody {
                experiment_id: args.experiment_id.to_uppercase(),
                title: args.title,
                finding: args.finding,
                evidence: args.evidence,
                improvement: args.improvement,
                remaining_debt: args.debt,
                next_move: args.next_move,
                impact: args.impact,
            };
            client.publish_finding(slug, body, actor, &id).await
        }
    }
}
