# ADR 0016: Auditable steer-constrain-suspend-stop circuit breaker

## Status

Accepted — August 30, 2026

## Decision

Budgets are authoritative at four scopes:

- run and task-contract token/cost limits;
- mission token/cost limits;
- requester rolling 24-hour token/cost limits; and
- Corp rolling 24-hour token/cost limits.

The breaker also consumes explicit tool-progress events. Repeated identical tools and no-progress
events are bounded, while events marked as healthy human conversation do not advance loop
counters.

Pressure produces monotonic stages:

1. `steer` at 75%;
2. `constrain` at 90%;
3. `suspend` at 100%; and
4. `stop` at 110%.

Each transition creates an immutable incident and a durable runner command. The stop stage
terminates the supervised process. Earlier stages are delivered as policy direction; suspend also
marks the run as waiting for input.

## Consequences

- Budget and loop decisions are explainable from recorded inputs.
- Healthy conversation does not look like a stuck tool loop.
- Adapters may provide stronger pause semantics over time; all receive the same policy stage.
- Bounded task-graph depth and node limits remain the recursion boundary.
