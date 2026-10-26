# Runtime deployment boundaries

The distributed runtime owns project execution, durable research state and guidance. Its current
packaging combines the HTTP server, supervisor and native provider processes. The local Compose
configuration is a development starting point, not an isolation boundary between hostile projects.

## State and continuity

The canonical database stores project state, research evidence, events and guidance. Workspaces,
provider continuation files and generated deliverables also require durable storage. A Markdown
export projects part of that state; it is not a full instance backup or a complete project migration.

Use one shared encryption key for processes accessing the same encrypted records, keep the key
outside source control, and retain controlled recovery material separately from database backups.

Database leases protect coordinator ownership. Multiple application replicas additionally need
validated workspace access, live-message routing, event replay and recovery on their target platform.
Do not infer seamless failover or uninterrupted agent execution from a successful health check.

## Execution environment

Prepare each project's repository, instructions, data, dependencies and toolchains explicitly.
Check a representative experiment before declaring the environment ready. Provider availability
does not establish that a project's tests or experiments can run.

The default image contains native provider binaries and basic utilities. Language interpreters
used by image-build stages are not automatically present in the final runtime image.

Untrusted execution needs a boundary that separates the project from the control plane and other
projects. Filesystem permissions, tool policies, container isolation and VM isolation provide
different guarantees. Never silently disable required protections to make an environment start.

## Interface and identity

Use authenticated TLS connections and project membership for team deployments. Configure exact
host/origin allowlists for the deployed endpoints. Static local-development tokens and example
database credentials are not production identity.

Infrastructure configuration and operational runbooks belong to the operator's private repository.
Pin the public runtime revision and image digest there. Reusable runtime interfaces and their
compatibility tests belong here, without organization-specific deployment values.

## Validation

Before relying on a deployment, verify both provider adapters, project isolation, guidance
incorporation, interrupted-work recovery, complete evidence reading and output retrieval. Exercise
backup restoration and application rollback against the chosen environment. Test results from one
local instance do not establish those guarantees for another deployment.
