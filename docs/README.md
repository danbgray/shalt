# shalt — technical documentation

`shalt` turns a plain-English request into formal, checkable obligations, has a human approve
them, and then drives isolated agents to satisfy them — recording in a durable ledger which
obligations are upheld and against exactly what wording.

## Reading order

| document | what it covers |
|---|---|
| [architecture.md](architecture.md) | the problem being solved, core concepts, module map, the pipeline end to end |
| [isolation.md](isolation.md) | the threat model, the three enforcement layers, and what each one actually stops |
| [ledger.md](ledger.md) | status state machine, full JSON schema, the invariants that make "green" mean something |
| [identity.md](identity.md) | scenario ids, canonical hashing, and why green expires |
| [runners.md](runners.md) | language agnosticism: config reference, report adapters, adding a stack |
| [hierarchy.md](hierarchy.md) | epic → story → task, user-story grammar, and the derived diagrams |
| [mutation.md](mutation.md) | mutation-testing the oracle: operators, the two signals, and the limits |
| [backends.md](backends.md) | the Backend protocol and the three adapters |
| [cli.md](cli.md) | complete command reference and exit codes |
| [limitations.md](limitations.md) | what this does not do, and what would break it |

Generated diagrams live in [diagrams/](diagrams/) and are produced by `shalt diagrams`.

Two worked examples: [`examples/invoice`](../examples/invoice) runs the whole agent loop offline
with no API key or toolchain, and [`examples/rust-billing`](../examples/rust-billing) is a real
Cargo project driven through cucumber-rs — the test that proved the language-agnosticism claim,
and found two bugs doing it.

## One-paragraph summary for the impatient

Four zones (`spec/`, `steps/`, `contract/`, `src/`), each writable by exactly one agent role.
Each role runs in a staging directory containing only the zones it may read, so the implementer
cannot see the tests it must pass and the stepwright cannot see the implementation. A turn that
writes outside its zone is rejected and rolled back. Every scenario carries a durable id stamped
at approval, and its upheld status is bound to a canonical hash of its meaning — change the
meaning and the status expires rather than silently carrying over. A scenario with no test bound
to it is never counted as upheld. `shalt mutate` then breaks the implementation to check whether
the scenarios actually notice, because a green suite written and graded by the same model proves
consistency, not correctness.
