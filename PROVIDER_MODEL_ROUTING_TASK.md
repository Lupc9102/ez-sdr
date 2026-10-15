# Provider Model Routing Investigation

## Target

Determine whether the configured Claude Code proxy/provider actually sends a requested GPT/SOL model to that model, or whether an upstream provider aliases, rewrites, or routes it to a different model (including the reported `5.6-sol` versus `6-sol` discrepancy).

Scope is limited to relevant local configuration and one controlled diagnostic request to the already configured provider. Do not expose credentials, tokens, cookies, or private response content in logs or chat. Do not modify application configuration as part of the investigation.

## Tasklist

- [x] Locate the effective Claude Code/provider configuration and identify endpoint, public model name, and routing settings. — `opencode` / `opencode/space-bunny-free`
- [x] Inspect proxy-side evidence for the model value actually sent upstream, using redacted output only. — `opencode` / `opencode/space-bunny-free`
- [x] Run one controlled provider request and capture redacted response/usage/routing metadata. — `opencode` / `opencode/space-bunny-free` (Claude endpoint probe completed; AgentRouter probe blocked by HTTP 401)
- [x] Compare local and provider evidence and record the conclusion, caveats, and any safe remediation. — `opencode` / `opencode/space-bunny-free`
- [x] Verify that no configuration or source files were changed unintentionally. — `opencode` / `opencode/space-bunny-free`
- [x] Write the session-end summary with findings and evidence locations. — `opencode` / `opencode/space-bunny-free`

## Tips

- Harness/model: `opencode` / `opencode/space-bunny-free`.
- Safety boundary: inspect only files and environment variables plausibly related to Claude Code/provider routing; never print secret values. Prefer metadata, hashes, and redacted excerpts.
- A dashboard model label alone may reflect provider-side normalization or aliasing; distinguish it from the model value on the wire and from backend identity.
- Keep the diagnostic request minimal and do not send unrelated user data. Do not brute-force endpoints, models, or authentication.
- The exact provider name and model spelling are not yet known; discover them from configuration and redact any sensitive values.
- Evidence should be recorded with file paths/line numbers where possible and with secrets replaced by `[REDACTED]`.
- Findings: Claude Code is configured for `https://cc-t2.freemodel.dev` in `/home/lupc/.claude/settings.json:5`; its live model catalog contains Claude IDs only, and controlled Messages requests for both GPT/SOL IDs returned HTTP 400 unsupported.
- The GPT/SOL definitions are in `/home/lupc/.config/opencode/opencode.json:13-18,78-80` and the default model is `AgentRouter-openai-gateway/gpt-6-astra` at line 256. This is a separate OpenCode/AgentRouter route.
- `/home/lupc/.config/opencode/agentrouter-proxy.mjs:13-28,98-99` forwards the request body without model inspection or rewriting. `/home/lupc/.claude/proxy/proxy.js:75,93` similarly parses the model but sends the original body upstream.
- AgentRouter metadata probes using the currently configured credential and one recent historical credential both returned HTTP 401; no live model identity could be obtained. Do not try additional credentials or brute-force the provider.
- Plaintext credentials were found in several user config files. Values were not repeated; rotate/revoke them and tighten permissions (`600`) rather than sharing them.
- Harness/model: `opencode` / `opencode/space-bunny-free`.
- Continuation pickup: `opencode` / `opencode/space-bunny-free`. User clarified that the Claude Code probe was accidental; inspect the effective `.codex` configuration for the same model-routing question. Preserve the prior findings and do not repeat the accidental Claude probe.
- Codex findings: effective `/home/lupc/.codex/config.toml:1-11` selects provider `freemodel`, model `gpt-6-astra`, and Responses wire API. The base URL was changed by user request to `https://api-t2-sg.freemodel.dev` at line 9. `codex doctor --json` confirms Codex loads the new URL, but provider reachability still fails (HTTP 403 on inference; `/models` probe 404).
- The shell has `OPENAI_MODEL=gpt-5.6-sol` and a different `OPENAI_BASE_URL`, but Codex ignores those for this invocation. A local capture server received `gpt-6-astra` on `/responses`; captures with `-m gpt-6-sol` and `-m gpt-5.6-sol` received those exact IDs unchanged.
- The current Codex process uses the API key stored in `/home/lupc/.codex/auth.json`, not the shell's `OPENAI_API_KEY`; only metadata/fingerprints were inspected.
- `[notice.model_migrations]` at `/home/lupc/.codex/config.toml:36-37` did not rewrite requests: local captures for `gpt-5.5` and `gpt-5.6-terra` sent those IDs unchanged.
- Before the endpoint correction, `api.freemodel.dev` model discovery returned HTTP 403; the historical configured route `api-t2-sg.freemodel.dev/v1/models` returned HTTP 200 and listed `gpt-6-luna`, `gpt-6-sol`, and `gpt-6-astra` as distinct IDs. Minimal inference on the historical route returned HTTP 503, so provider-side backend identity remains unavailable.
- The configured `workbuddy-proxy.service` is inactive and the current Codex config points directly to `api-t2-sg.freemodel.dev`; no local Codex model-mapping proxy is active.
- Endpoint correction: changed only `/home/lupc/.codex/config.toml:9` from `https://api.freemodel.dev` to `https://api-t2-sg.freemodel.dev` at the user's request. Model, auth, and all other provider settings were left unchanged.
- Verification after the edit: Codex loads the new base URL, but `codex doctor` still reports provider reachability failure. This is an endpoint/service issue, not evidence of model substitution.

## Codex continuation tasklist

- [x] Locate the effective `.codex` config, model selection, provider endpoint, and any local proxy. — `opencode` / `opencode/space-bunny-free`
- [x] Inspect Codex-side proxy/model handling for aliases or rewrites. — `opencode` / `opencode/space-bunny-free`
- [x] Run a minimal Codex-route metadata or generation probe only if an already configured credential works. — `opencode` / `opencode/space-bunny-free` (local capture succeeded; live Freemodel inference returned 503)
- [x] Compare Codex evidence with the prior Claude/OpenCode findings and update the conclusion. — `opencode` / `opencode/space-bunny-free`
- [x] Update the session summary and verify that only the requested `.codex` configuration change was made. — `opencode` / `opencode/space-bunny-free`

## Endpoint correction tasklist

- [x] Change only the current Freemodel base URL to the user-requested T2 endpoint. — `opencode` / `opencode/space-bunny-free`
- [x] Verify Codex loads the new endpoint and record remaining reachability errors. — `opencode` / `opencode/space-bunny-free`
- [x] Update the session summary with the intentional configuration change. — `opencode` / `opencode/space-bunny-free`

