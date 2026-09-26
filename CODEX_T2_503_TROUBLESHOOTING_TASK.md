# Codex T2 503 Troubleshooting

## Target

Diagnose the `503 Service temporarily unavailable` from `https://api-t2-sg.freemodel.dev/responses` after switching Codex from the legacy Freemodel endpoint. Determine whether the failure is caused by the base path, authentication, selected model, or upstream availability; apply a safe configuration fix only if evidence supports one.

## Tasklist

- [x] Reproduce the failure and capture redacted status/error metadata. — `opencode` / `opencode/space-bunny-free`
- [x] Compare `/responses`, `/v1/responses`, and model discovery using minimal requests. — `opencode` / `opencode/space-bunny-free`
- [x] Compare the configured model with other models advertised by T2. — `opencode` / `opencode/space-bunny-free`
- [x] Apply and verify the smallest safe fix, or report an upstream outage if no local fix exists. — `opencode` / `opencode/space-bunny-free`
- [x] Write the session-end summary and verify configuration changes. — `opencode` / `opencode/space-bunny-free`

## Follow-up

- [x] Reassess the raw API probes knowing that FreeModel may reject every non-Codex request format. — `opencode` / `opencode/space-bunny-free`
- [x] Capture and test the exact request emitted by Codex without printing its payload or credentials. — `opencode` / `opencode/space-bunny-free`
- [x] Correct the session conclusion and summary based on Codex-native evidence. — `opencode` / `opencode/space-bunny-free`

## Tips

- 2026-09-24 — Harness/model: `opencode` / `opencode/space-bunny-free`.
- Safety: never print API keys or passwords. Read credentials only inside redacted probe scripts.
- The current Codex model is `gpt-6-sol`; T2 advertises `gpt-6-luna`, `gpt-6-sol`, and `gpt-6-astra` as distinct IDs.
- The user explicitly reported `https://api-t2-sg.freemodel.dev/responses` returning HTTP 503.
- Earlier raw-payload probe results are superseded because FreeModel does not accept ordinary non-Codex request formats.
- Authentication behavior remains distinct from inference health: missing and invalid credentials return HTTP 403 and 401 respectively, while the configured key passes authentication.
- The main Freemodel catalog exposes a different generation (`gpt-5.6-*`); raw probes against that catalog were also superseded and are not used in the final conclusion.
- Exact Codex-native probes establish that neither T2 nor VIP currently serves this account's 6-series requests. The requested T2 configuration remains intact.
- Follow-up correction: raw non-Codex payloads are not valid health tests for this provider. Exact Codex-generated requests to every T2 model still returned HTTP 503.
- User supplied another candidate endpoint: `https://vip-sg.freemodel.dev`.
- VIP model discovery returns HTTP 200 with the same three T2 model IDs. Exact Codex requests still return HTTP 503 for `gpt-6-sol`, `gpt-6-astra`, and `gpt-6-luna`, on both `/responses` and `/v1/responses`.
