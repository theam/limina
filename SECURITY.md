# Security policy

Limina can run code and use credentials supplied to a research project. Treat project repositories,
retrieved documents and agent output as untrusted input. Application permissions and an execution
sandbox address different parts of that boundary.

## Reporting a vulnerability

Do not include credentials, customer data or exploit details in public issues. Use GitHub's private
vulnerability reporting when available. Otherwise ask a maintainer to establish a private reporting
channel without including sensitive details in that request.

## Public repository boundary

The repository contains reusable runtime code, templates, documentation and synthetic tests.
Organization-specific infrastructure, account identifiers, domains, identity configuration,
operational histories and live project data belong in a separate private repository or data store.
Never commit provider credentials, encryption keys, local state or infrastructure state.

The ignored-file rules reduce accidental commits; they do not remove sensitive material already
tracked by Git. Review the final tree and new commit history before publishing a contribution.

## Runtime boundary

- Authentication and project membership must protect every project operation.
- Agents receive only the capabilities and resources required by their project.
- A local development configuration is not a production security configuration.
- Review [deployment boundaries](docs/deployment-boundaries.md) before exposing an instance.

Back up the canonical database, required project files and encryption-key recovery material through
separate controlled mechanisms. Exercise restoration; a knowledge export is not a complete backup.
