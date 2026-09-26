# LRPT Decode Audit Task

## Target
Deep audit of the `lrpt-decode` Rust crate for practical and theoretical correctness: QPSK handling, frame synchronization, CCSDS framing, Reed-Solomon, image assembly/output, runtime behavior, error handling, and integration assumptions. Produce evidence-based findings with file/line references; do not modify source code.

## Tasklist
- [ ] Map crate structure and execution/data flow.
- [ ] Audit signal/QPSK demodulation and bit/byte semantics.
- [ ] Audit frame synchronization and CCSDS packet handling.
- [ ] Audit Reed-Solomon and image reconstruction/output behavior.
- [ ] Run feasible tests/static checks and assess runtime practicality.
- [ ] Report findings and grade to parent agent.

## Tips
- Active agent: Codex collaboration subagent; model identifier: GPT-6 (gpt-6-astra).
- Preserve exact file:line evidence; classify severity and explain practical impact.
