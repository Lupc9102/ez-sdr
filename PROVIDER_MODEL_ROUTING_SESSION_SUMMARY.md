# Provider Model Routing Investigation — Session Summary

Date: 2026-09-24
Harness/model: `opencode` / `opencode/space-bunny-free`

## Scope and safety

Inspected only user-readable Claude Code/OpenCode/provider proxy configuration and ran minimal metadata or one-token diagnostic requests against endpoints already present in the local configuration. No application/source files were modified; the only intentional configuration change was the user-requested Codex endpoint update. No secret values were printed or used in chat. The user-provided sudo password was not used.

## Actions and evidence

1. Read `/home/lupc/.claude/settings.json`.
   - Claude Code is configured for `https://cc-t2.freemodel.dev` at line 5.
   - Its configured model is `sonnet` at line 12.
   - The live `/v1/models` catalog returned eight Claude model IDs and no GPT/SOL IDs.
   - Minimal Anthropic Messages requests for both `gpt-6-astra` and `gpt-5.6-sol` returned HTTP 400 with an unsupported-model error.

2. Read the OpenCode configuration.
   - `/home/lupc/.config/opencode/opencode.json:13-18` and `:78-80` define `gpt-6-astra` and `gpt-5.6-sol` as separate model IDs.
   - `/home/lupc/.config/opencode/opencode.json:256` selects `AgentRouter-openai-gateway/gpt-6-astra` as the default.
   - This is a separate OpenCode/AgentRouter route, not the Claude Code route above.

3. Inspected both local proxies.
   - `/home/lupc/.config/opencode/agentrouter-proxy.mjs:13-28,98-99` buffers and forwards the request body without parsing or rewriting the model.
   - `/home/lupc/.claude/proxy/proxy.js:75,93` parses the model but sends the original body upstream; the parsed value is not used for routing.
   - The AgentRouter proxy is active on `127.0.0.1:8788` and forwards to `https://agentrouter.org` by default.
   - The separate Claude proxy is active on port `8080`; its health endpoint reported `0/8` providers working, but that health check uses a Claude model and does not establish GPT/SOL routing behavior.

4. Tested AgentRouter authentication without brute force.
   - The currently configured AgentRouter credential and one recent historical configured credential both returned HTTP 401 for `/v1/models` using both bearer and `x-api-key` header styles.
   - Therefore a live authenticated GPT/SOL completion could not be captured. The 401 prevents a definitive provider-side model-identity test; it is not evidence of model substitution.

## Conclusion

The local proxy code is not rewriting `gpt-6-astra` to `gpt-5.6-sol`. If the dashboard genuinely records a request sent as `gpt-6-astra` but bills/routes it as `gpt-5.6-sol`, the mismatch occurs at the AgentRouter/provider layer, unless the client itself sent the second model ID. The dashboard label and price inversion make provider-side aliasing plausible, but do not prove the backend identity without a successful request ID and raw usage record.

The currently configured Claude Code endpoint is not the source of the GPT/SOL traffic: it rejects both GPT/SOL IDs and advertises only Claude models. The GPT/SOL configuration belongs to the separate OpenCode/AgentRouter path.

## Security follow-up

Several user configuration files contain plaintext provider credentials. Values were not repeated here. Revoke/rotate those credentials, rotate the sudo password shared in chat, and restrict sensitive config files to mode `600`. Do not paste replacement keys into chat.

## Codex continuation

The user clarified that the earlier Claude Code probe was accidental and requested the same check for Codex.

- Effective `/home/lupc/.codex/config.toml:1-11` selects provider `freemodel`, model `gpt-6-astra`, and the Responses API. The base URL was intentionally changed to `https://api-t2-sg.freemodel.dev`.
- `codex doctor --json` independently reported the same effective model/provider. The shell's `OPENAI_MODEL=gpt-5.6-sol` and `OPENAI_BASE_URL=https://cc-t2.freemodel.dev` were not used for the Codex request.
- A local capture server received `gpt-6-astra` on `/responses`; explicit `-m gpt-6-sol` and `-m gpt-5.6-sol` captures received those exact IDs unchanged. The current Codex process used the API key stored in `auth.json`, not the shell's `OPENAI_API_KEY`.
- The `[notice.model_migrations]` entry at `/home/lupc/.codex/config.toml:36-37` is not a request rewrite: local captures for `gpt-5.5` and `gpt-5.6-terra` sent those IDs unchanged.
- Before the endpoint correction, the `api.freemodel.dev` model-discovery route returned HTTP 403. The historical `api-t2-sg.freemodel.dev/v1/models` route returned HTTP 200 and listed `gpt-6-luna`, `gpt-6-sol`, and `gpt-6-astra` as distinct IDs. A minimal inference request to the historical route returned HTTP 503, so no provider response model metadata was available.
- The configured `workbuddy-proxy.service` is inactive; the current Codex config points directly to `api-t2-sg.freemodel.dev` and has no active local model-mapping proxy.

### Updated conclusion

Codex itself is not aliasing `gpt-6-astra`, `gpt-6-sol`, or `gpt-5.6-sol`; local captures prove pass-through. The current Codex configuration is also not actually selecting `gpt-6-sol`: it selects `gpt-6-astra`. If a dashboard attributes a request to `gpt-5.6-sol`, the remaining explanations are an upstream Freemodel alias/router, a different historical Codex route/session, or a mismatch between the dashboard account and the current config. The live provider check is currently blocked by endpoint errors (403/503), so backend identity is not proven.

## Endpoint correction

- Changed only `/home/lupc/.codex/config.toml:9` from `https://api.freemodel.dev` to `https://api-t2-sg.freemodel.dev` at the user's request.
- Model, authentication, reasoning effort, wire API, and all unrelated settings were left unchanged.
- `codex doctor --json` confirms the new URL is loaded. It still reports provider reachability failure: HTTP 403 on the Responses route and HTTP 404 on its generic `/models` probe. The separate `/v1/models` route on the T2 host was previously observed returning HTTP 200.

## Repository state

The repository was already heavily dirty before/during this investigation. The only investigation-specific repository files created were:

- `PROVIDER_MODEL_ROUTING_TASK.md`
- `PROVIDER_MODEL_ROUTING_SESSION_SUMMARY.md`

The only intentional external configuration change was `/home/lupc/.codex/config.toml:9`, documented above. No commit was made.

