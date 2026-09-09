# Connect a project and coding agent

ECorp saves an execution connection for a project room: repository, source
revision, coding agent and execution machine. Selecting a connection does not
start a mission.

## In the application

1. Open **New mission**, then **Connect a repository or coding agent**.
2. Choose the execution machine. ECorp uses an existing enrolled runner; adding
   a repository does not create another Docker stack.
3. Choose the repository:
   - **GitHub repository:** browse repositories available to the selected native
     GitHub account, or enter an authorized repository URL.
   - **Existing local checkout:** an owner/admin can connect a Git checkout under
     the machine operator's approved source roots.
   - **Save the current repository:** retain the already-advertised source.
4. Choose **GitHub Copilot**, **Codex** or **Claude Code**, then **Connect and test**.
5. Complete native sign-in if needed. Follow the provider's page; never paste
   a password or API key into ECorp. Claude's remote sign-in may ask for the
   one-time authorization code returned by its native browser flow.
6. Choose **Use** on the saved connection. Describe the outcome and review the
   exact source revision before starting the mission.

Ready means the runner accepted the repository and native agent connection
checks. It is not a claim that an application has already been built or that a
provider will never reject a later request. Model execution and verification
still produce their own evidence.

Saved connections remain visible when their machine is offline. ECorp does not
silently switch to another repository, account or coding agent. Reconnect the
selected machine, or deliberately select another connection.

## Native accounts

GitHub repository access and coding-agent authentication are separate native
connections. A connected GitHub repository does not imply that a coding agent
is signed in.

- Personal GitHub configuration is actor-scoped on the selected runner.
  `GH_CONFIG_DIR` is configuration separation, not a separate operating-system
  identity or an isolated OS keychain.
- Using the machine's existing GitHub account or importing a host checkout
  requires an owner/admin. Those operations do not log out, switch or overwrite
  that machine's native account configuration.
- Native sign-in URLs, codes and repository discovery results are returned only
  to the operator who requested them. Shared room events carry refresh metadata,
  not the private report or credentials.
- Native credential-store fallback and environment-based credential delivery
  remain reduced assurance. They are not advertised as OS containment.
- The same native configuration is used for connection checks, execution and
  resume. Copilot uses the repository's supported SDK/CLI pair, not an
  incompatible executable discovered elsewhere on `PATH`.
- Executable selection and native account selection are independent. An
  installed Codex or Claude executable can use a private connection profile.
  Reusing the machine's existing agent sign-in is an explicit owner/admin option;
  it does not authorize the application to change that machine account.

This feature does **not** turn local Alice/Bob/Eve demo operators into GitHub
sign-in for ECorp itself. Production human authentication remains the configured
OIDC boundary.

## Machine operator settings

The runner supports these optional settings in addition to its existing
enrollment, server and workspace configuration:

| Runner option | Environment variable | Purpose |
| --- | --- | --- |
| `--connections-directory` | `CRONY_CONNECTIONS_DIRECTORY` | Private native connection state, outside source repositories |
| repeatable `--repository-root` | `CRONY_REPOSITORY_ROOTS` (semicolon-separated) | Approved roots for importing existing local Git checkouts |
| `--github-command` | `CRONY_GITHUB_COMMAND` | The operator's trusted GitHub CLI executable |

Without an explicit connections directory, the runner uses an operator-local
data directory namespaced by server, Corp and runner. By default, local checkout
imports are limited to the configured source repository. Grant additional local
roots once at runner setup; a browser request cannot widen that filesystem
boundary. GitHub clones use managed private source storage instead.
The supported Windows launcher retains these non-secret settings in its existing
local configuration so a normal restart uses the same connection directory and
approved local roots.

The server sends only fixed, typed setup operations. It cannot send an arbitrary
shell command, executable, environment map or credential value through this API.
Native child-process ownership and bounded operation lifetimes still apply.

## Retained work

Each run stores its execution connection identity. New missions use the selected
checked revision; a later branch refresh does not rewrite old run history.
Resume and verifier-only recovery keep the original connection and source pin.
Model sign-in is not required merely to resolve previously accepted source for
verification. Failed checks do not delete retained worktrees.

Native result receipts persist until the server acknowledges them and are
replayed after reconnect. Late acknowledgements cannot select an older
configuration over a newer one.

## Use the same connection for Factory

Direct missions and Factory can use the same saved GitHub repository and native
coding-agent connection. The trusted controller selects it explicitly:

```powershell
$env:ECORP_FACTORY_WORKSPACE_CONNECTION_ID = '<saved-connection-id>'
$env:ECORP_FACTORY_REPOSITORY = 'your-team/your-application'
$env:ECORP_FACTORY_SOURCE_BASE_REF = 'main'
```

For `crony factory` or `crony factory-watch`, the equivalent option is
`--workspace-connection-id <saved-connection-id>`. Select the same repository,
source ref and coding agent as the saved connection. The existing authorized
room-connections API returns the connection ID; the exact connection read
returns only its shared configuration/status, never native sign-in details.

For a saved connection, new intake uses its runner-checked immutable source
identity. The controller does not need another manually cloned source checkout.
The server rechecks the current connection, source, room membership and live
runner before admitting the plan. A Ready connection is not authorization to
run another account's environment or a different revision.

Factory persists the connection in its claimed policy and carries it through
every task and run. Recovery must retain that original connection option; it
cannot substitute a new connection or silently fall back to legacy routing.
The supported launcher remembers these non-secret Factory settings. Changing
a running controller's configuration still uses explicit restart, not an
implicit replacement of the runner or its source checkout.

Omitting the connection option preserves legacy Factory routing through its
configured source checkout. Existing unbound policy snapshots are not rewritten.
This does not change GitHub Project eligibility, outcome review, publication
authorization or the separate authorization required for merge/deployment.

## Validation scope

Connection unit/SQLx tests prove their named metadata, authority and lifecycle
cases. Native protocol fixtures are not vendor inference. The complete
acceptance for issue #198 additionally requires the real browser-to-server-to-
runner connection flow and an application build against a non-ECorp target.
Do not claim that acceptance from a green picker, authenticated status response
or compile alone.

The [September 9 native acceptance report](evidence/2026-09-09-project-connections-native-acceptance.md)
records that flow for a real Copilot-built application, including its exported
source and browser review. It distinguishes private Codex/Claude sign-in checks
from inference, and leaves the broader Factory and real-GitHub publication work
under #145 / #63 open.
