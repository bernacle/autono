# Security policy

Autono Runtime v0 executes commands with the daemon user's privileges. It is
**not a sandbox**. Only submit commands and projects you trust.

Engineering workflows also execute installed coding CLIs and developer-configured
verification commands. Provider permission restrictions and workspace mutation
audits reduce accidental writes; they are not a security boundary against a
malicious agent, repository, test, or local user. Worktree audits exclude ignored
files and cannot detect transient/reverted or external writes. See
[the engineering boundaries](docs/engineering.md) before approving a plan.
Autono uses CLI login without reading authentication files. Provider output,
prompts, diffs, and verification logs are retained locally and can contain
sensitive project data. Protect the engineering directory with the rest of the
Autono home.

Preflight checks environment variable nonempty presence by name only and never
retains or exposes their values or reads credential files. Human resolutions must
contain nonsecret decisions or
setup evidence. Configure credentials externally. Permission policy is snapshotted
and user-only; explicit exact-scope approvals never broaden provider sandboxes.
The broker governs declared operations, not arbitrary code/syscalls. See
[preflight limits](docs/preflight.md) before using unattended execution.

The daemon listens on IPv4 loopback only. Every route requires a random 256-bit
bearer token, published in `daemon.json` inside the user data directory. Tokens
rotate on startup. Browser Origin headers and nonmatching Host headers are
rejected; no cookies, query-string tokens, or CORS are supported. Authenticated request bodies and HTTP connection drain have bounded deadlines.
Run/submission admission and the HTTP connection set are bounded independently.
SSE requires an
Authorization header, so browser integrations need a trusted native bridge or
equivalent authenticated fetch, not an unauthenticated EventSource URL.

On Unix the Autono home is owner-only (`0700`), with private token/lock files
(`0600`). Use a dedicated Autono home, not a shared directory. Windows currently
relies on the user profile's inherited filesystem permissions; additional ACL
hardening and full Windows process-tree supervision are future work. Do not
proxy the API, bind it externally, or share daemon metadata. Authentication is
local-user protection, not multi-user isolation. Another process running as the
same OS user can read the token and is inside the trust boundary.

Command arguments, explicit environment overrides, working directories, stdout,
and stderr are stored locally without encryption. They may contain credentials
or sensitive source data. Status and event APIs expose this data to authorized
clients. Raw log replay can include terminal control sequences. Protect backups
and use structured `autono logs --events` when inspecting untrusted output.

After abrupt daemon death, processes may survive. Restart marks unfinished runs
`INTERRUPTED`, never adopts or kills stored PIDs, and cannot undo command side
effects. Detached descendants can escape Unix process groups. See the README's
recovery limitations before retrying interrupted work.

For a suspected vulnerability, use the repository's GitHub **Security → Report a
vulnerability** private reporting facility when enabled. If unavailable, open an
issue requesting a private reporting channel without exploit details, tokens,
logs, or personal data. Do not post sensitive reproductions publicly. Runtime
v0 is pre-1.0; security fixes target the latest main branch, with no guaranteed
backport schedule.
