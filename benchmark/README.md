# Beam benchmark

`beam-benchmark` measures complete Beam task round trips. The same process acts as
the sending app on proxy 1 and the receiving app on proxy 2:

```text
app1 -> proxy1 -> broker -> proxy2 -> app2
app1 <- proxy1 <- broker <- proxy2 <- result
```

Start the development stack in one terminal:

```sh
./dev/beamdev start
```

Then run the benchmark in another terminal:

```sh
cargo run --release -p beam-benchmark -- --requests 1000 --concurrency 20
```

The defaults match `beamdev`: `localhost:8081`, `localhost:8082`,
`app1.proxy1.broker`, `app2.proxy2.broker`, and API key `App1Secret`. Run with
`--help` to override any of these, change payload size, or disable warm-up.

Each measured cycle posts one task, retrieves it as the receiving app, submits a
successful result, and retrieves that result as the sending app. The final report
contains end-to-end throughput and latency percentiles as well as timings for the
individual stages.

