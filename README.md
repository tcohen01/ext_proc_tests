# ext_proc_tests

External Processor Stream Reuse artificial performance comparison test

| Scenario | Hardware | OS | Server Command | Client Command | RPS |
|----------|----------|----|----------------|----------------|-----|
| No Reuse | i7-8700K @ 4.8 Ghz | Windows 10 2022H2 | run_server | bench_client bench/fixtures/simple.json | 14,140 |
| Infinite Reuse | i7-8700K @ 4.8 Ghz | Windows 10 2022H2 | run_server | bench_client --reuse-streams bench/fixtures/simple.json | 31,650 |
| 100 Transaction Reuse | i7-8700K @ 4.8 Ghz | Windows 10 2022H2 |  run_server | bench client --reuse-streams --stream-max-handle 100 bench/fixtures/simple.json | 32,177 |
| No Reuse | i7-8700K @ 4.9 Ghz | Linux Mint 21.1 | run_server | bench_client bench/fixtures/simple.json | 29,888 |
| Infinite Reuse | i7-8700K @ 4.9 Ghz | Linux Mint 21.1 | run_server | bench_client --reuse-streams bench/fixtures/simple.json | 56,765 |
| 100 Transaction Reuse | i7-8700K @ 4.9 Ghz | Linux Mint 21.1 |  run_server | bench client --reuse-streams --stream-max-handle 100 bench/fixtures/simple.json | 57,242 |

A few things to note:

* Client doesn't have and cannot have a way to cut a transaction short - all requests sent by the client must be responded to.
** Aborting a transaction in a stream will then require waiting for and ignoring outstanding responses
** This will be more complicated for responses that don't have 1-1 relation to a request (many responses for one request, may be an option later for streaming large replacement bodies from the External Processor server to the client) - Explicitly state which request number within the transaction the response is referring to and how many responses for that request?
* RequestHeaders request should always be considered the start of a new transaction within a reused stream.