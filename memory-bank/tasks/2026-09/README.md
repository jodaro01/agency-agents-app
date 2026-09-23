# September 2026 — Linux AppImage unblocked; the release backlog comes into focus

## Shipped to `main`

**PR #102 — `fix(linux): stop AppImage bundle paths leaking into child processes; show real error text`**
Closes **#94** and **#92**. See `decisions.md` 2026-09-12 for the architecture call and `activeContext.md` for the
live picture.

### How it was verified (the bar worth keeping)
1. **Read the artifact, not the theory.** Downloaded the shipped v0.3.0 amd64 AppImage, computed the squashfs
   offset from the ELF header (`e_shoff + e_shentsize × e_shnum`), unsquashed it, and read `AppRun`,
   `apprun-hooks/linuxdeploy-plugin-gtk.sh` and the strings inside `AppRun.wrapped`. That is how we know
   `LD_LIBRARY_PATH` is set with bundle dirs first, with no original saved.
2. **Reproduced the failure independently.** On `ssh scratch` (Ubuntu 26.04 aarch64) with a genuine Ubuntu 22.04
   arm64 `libnghttp2` from `ports.ubuntu.com` placed on `LD_LIBRARY_PATH`: `git clone` fails with the exact symbol
   the Arch reporter saw. A clean environment clones fine. **So the bug is not distro-specific.**
3. **Proved the fix end to end.** Compiled the real `util::proc` helper into a small crate on the VM and cloned
   twice in the same poisoned environment — without `sanitize()`: fail; with it: success.
4. **Pinned it with tests.** Seven unit tests: bundle entries dropped, host entries kept, wholly-bundled lists
   removed, untouched values passed through unchanged, no-op without `$APPDIR`, and one that spawns a real child
   and asserts it cannot see the bundle path.

### Process notes worth remembering
- **`npm run check` is 0 errors on clean `main`.** Take that baseline *before* editing; then every error is yours.
  Doing so here turned a confusing "90 errors" into "one brace I broke".
- **Scripted multi-file edits need the diff read.** The `String(e)` → `errorText(e)` sweep across 16 files
  introduced four separate breakages (Svelte imports placed above `<script>`, a brace-collapse that cascaded, a
  mangled multiline import, two missed sites). All were caught by tooling; none by inspection. Reading the finished
  diff then caught three more things tooling could not see: duplicate imports, a raw `e.code` fallthrough, and
  three redundant branches.
- **`cargo fmt` is not clean on `main`** (PR #100 is the fix). Format only files you add; reformatting a file you
  touched buries a 3-line change under ~50 lines of churn and collides with #100.
- **Deliberately not done:** `reportableToastError()` already exists and would add a "Report to Agency Agents"
  button at the toast sites. That is a UX decision, not a bug fix, so it was left for its own PR.

## Not done — the actual priority
`main` has gone unshipped since **v0.3.0 (2026-07-05)** while 16 mergeable PRs and 16 issues accumulated. Fixes that
do not ship do not count. The cheapest large win available is a **v0.3.1**: merge the small, already-written
platform fixes (#69 Windows WebView2 +3 lines, #85 console flash, #99 Linux AppArmor), then tag.


## Native Paperclip bridge prototype (2026-09-23)

- Isolated fork branch: `codex/paperclip-native-sync-2026-09-23`; pre-change snapshot: `backup/pre-paperclip-2026-09-23`.
- Native Tauri REST bridge uses Paperclip's browser-approved CLI challenge, requests board access for one selected company, keeps the pending bearer token in native memory, and stores the approved token in the OS Keychain.
- First slice reads a safe agent roster and active projects through typed DTOs; it deliberately does not edit agent instructions, adapter configuration, projects, or workspaces.
- Paperclip MCP health check returned 0.3.1 / authenticated / public; the three delegated Paperclip issues have no execution run.
- Local build and app verification remain pending because the local terminal tunnel is unavailable. Do not call this a completed mirror until the app is built and the Droplet connection is verified.


## Paperclip adapter/plugin compatibility check (2026-09-23)

- Live health reports Paperclip `0.3.1`, deployment mode `authenticated`, exposure `public`.
- The official `v0.3.1` adapter registry contains nine built-ins: `claude_local`, `codex_local`, `cursor`, `gemini_local`, `opencode_local`, `openclaw_gateway`, `pi_local`, `process`, and `http`. It has no built-in Hermes adapter.
- The v0.3.1 route tree has neither `/api/cli-auth/challenges` nor global `/api/adapters` / `/api/plugins` registry endpoints. Its company-scoped adapter route returns models for a given type, not the installed registry. Source: https://github.com/paperclipai/paperclip/tree/v0.3.1/server/src/routes
- Unauthenticated GETs to the live `/api/adapters` and `/api/plugins` returned 403; this alone does not prove route support.
- The available Paperclip MCP has no safe adapter/plugin inventory call. Do not use `list_agents` for inventory: its response included sensitive adapter configuration in a prior tool trace. Exact deployed assignments remain unverified.
- A native inventory/API bridge requires a compatible Paperclip upgrade or an authenticated, supported server-side route. The current PR #1 is blocked, draft only, and must not be merged or installed against v0.3.1.
- Security follow-up: live 0.3.1 is in the affected range of https://github.com/paperclipai/paperclip/security/advisories/GHSA-3pw3-v88x-xj24 unless a backport is present; patched upstream version is 2026.416.0. Have the operator verify/upgrade.
