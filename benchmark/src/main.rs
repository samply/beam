use std::{
    io::{self, IsTerminal, Write},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use beam_lib::{
    set_broker_id, AddressingId, BeamClient, BlockingOptions, FailureStrategy, MsgId, TaskRequest,
    TaskResult, WorkStatus,
};
use clap::Parser;
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(
    name = "beam-benchmark",
    about = "Benchmark complete task/result round trips through a Beam stack"
)]
struct Args {
    /// Number of measured task round trips.
    #[arg(short = 'n', long, default_value_t = 100)]
    requests: usize,

    /// Maximum number of task round trips in flight.
    #[arg(short = 'c', long, default_value_t = 10)]
    concurrency: usize,

    /// Unmeasured task round trips performed before the benchmark.
    #[arg(long, default_value_t = 5)]
    warmup: usize,

    /// Payload bytes placed in each task and result.
    #[arg(short = 's', long, default_value_t = 128)]
    payload_size: usize,

    /// Maximum time for one complete round trip, in seconds.
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    /// Sender's Beam proxy URL.
    #[arg(long, default_value = "http://localhost:8081")]
    sender_url: String,

    /// Receiver's Beam proxy URL.
    #[arg(long, default_value = "http://localhost:8082")]
    receiver_url: String,

    /// Beam ID used to submit tasks and collect results.
    #[arg(long, default_value = "app1.proxy1.broker")]
    sender_id: String,

    /// Beam ID used to receive tasks and submit results.
    #[arg(long, default_value = "app2.proxy2.broker")]
    receiver_id: String,

    /// API key accepted by both development proxies.
    #[arg(long, env = "APP_KEY", default_value = "App1Secret")]
    api_key: String,

    /// Broker ID used when validating strict Beam IDs.
    #[arg(long, env = "BROKER_ID", default_value = "broker")]
    broker_id: String,

    /// Time-to-live assigned to generated tasks.
    #[arg(long, default_value = "5m")]
    ttl: String,

    /// Disable ANSI colors and the updating progress line.
    #[arg(long)]
    no_color: bool,
}

#[derive(Clone)]
struct Config {
    sender: BeamClient,
    receiver: BeamClient,
    sender_id: AddressingId,
    receiver_id: AddressingId,
    run_id: String,
    payload: Arc<str>,
    ttl: Arc<str>,
    timeout: Duration,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BenchmarkBody {
    run_id: String,
    sequence: usize,
    data: String,
}

#[derive(Debug)]
struct Sample {
    post: Duration,
    delivery: Duration,
    answer: Duration,
    result_return: Duration,
    total: Duration,
}

#[derive(Debug)]
struct FailedSample {
    sequence: usize,
    elapsed: Duration,
    error: String,
}

type SampleResult = std::result::Result<Sample, FailedSample>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    validate_args(&args)?;
    set_broker_id(args.broker_id.clone());

    let sender_id = AddressingId::new(&args.sender_id)
        .with_context(|| format!("invalid sender Beam ID: {}", args.sender_id))?;
    let receiver_id = AddressingId::new(&args.receiver_id)
        .with_context(|| format!("invalid receiver Beam ID: {}", args.receiver_id))?;
    let sender_url = args.sender_url.parse().context("invalid sender URL")?;
    let receiver_url = args.receiver_url.parse().context("invalid receiver URL")?;
    let run_id = format!(
        "beam-benchmark-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    );
    let config = Arc::new(Config {
        sender: BeamClient::new(&sender_id, &args.api_key, sender_url),
        receiver: BeamClient::new(&receiver_id, &args.api_key, receiver_url),
        sender_id,
        receiver_id,
        run_id,
        payload: "x".repeat(args.payload_size).into(),
        ttl: args.ttl.into(),
        timeout: Duration::from_secs(args.timeout),
    });

    println!("\n  Beam task round-trip benchmark");
    println!("  {} -> {}", args.sender_id, args.receiver_id);
    println!(
        "  {} requests, {} concurrent, {} B payload\n",
        args.requests, args.concurrency, args.payload_size
    );

    if args.warmup > 0 {
        print!("  Warming up with {} requests...", args.warmup);
        io::stdout().flush()?;
        let warmup = run_batch(config.clone(), args.warmup, args.concurrency).await;
        let failed = warmup.iter().filter(|sample| sample.is_err()).count();
        if failed > 0 {
            println!(" failed ({failed}/{})", args.warmup);
            return Err(anyhow!(
                "warm-up did not complete; check that ./dev/beamdev start is running"
            ));
        }
        println!(" done");
    }

    let interactive = io::stdout().is_terminal() && !args.no_color;
    let started = Instant::now();
    let samples = run_batch_with_progress(
        config,
        args.requests,
        args.concurrency,
        interactive,
        started,
    )
    .await;
    let elapsed = started.elapsed();

    if interactive {
        print!("\r\x1b[2K");
    }
    print_report(&samples, elapsed, args.requests, interactive);
    Ok(())
}

fn validate_args(args: &Args) -> Result<()> {
    if args.requests == 0 {
        return Err(anyhow!("--requests must be greater than zero"));
    }
    if args.concurrency == 0 {
        return Err(anyhow!("--concurrency must be greater than zero"));
    }
    if args.timeout == 0 {
        return Err(anyhow!("--timeout must be greater than zero"));
    }
    Ok(())
}

async fn run_batch(config: Arc<Config>, count: usize, concurrency: usize) -> Vec<SampleResult> {
    stream::iter(0..count)
        .map(|sequence| run_timed_cycle(config.clone(), sequence))
        .buffer_unordered(concurrency)
        .collect()
        .await
}

async fn run_batch_with_progress(
    config: Arc<Config>,
    count: usize,
    concurrency: usize,
    interactive: bool,
    started: Instant,
) -> Vec<SampleResult> {
    let mut work = stream::iter(0..count)
        .map(|sequence| run_timed_cycle(config.clone(), sequence))
        .buffer_unordered(concurrency);
    let mut samples = Vec::with_capacity(count);
    let mut ticker = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            biased;
            sample = work.next() => match sample {
                Some(sample) => {
                    samples.push(sample);
                    if interactive {
                        draw_progress(samples.len(), count, &samples, started.elapsed());
                    }
                }
                None => break,
            },
            _ = ticker.tick(), if interactive => {
                draw_progress(samples.len(), count, &samples, started.elapsed());
            }
        }
    }
    samples
}

async fn run_timed_cycle(config: Arc<Config>, sequence: usize) -> SampleResult {
    let started = Instant::now();
    match tokio::time::timeout(config.timeout, run_cycle(&config, sequence, started)).await {
        Ok(Ok(sample)) => Ok(sample),
        Ok(Err(error)) => Err(FailedSample {
            sequence,
            elapsed: started.elapsed(),
            error: format!("{error:#}"),
        }),
        Err(_) => Err(FailedSample {
            sequence,
            elapsed: started.elapsed(),
            error: format!(
                "round trip timed out after {:.1}s",
                config.timeout.as_secs_f64()
            ),
        }),
    }
}

async fn run_cycle(config: &Config, sequence: usize, started: Instant) -> Result<Sample> {
    let task_id = MsgId::new();
    let body = BenchmarkBody {
        run_id: config.run_id.clone(),
        sequence,
        data: config.payload.to_string(),
    };
    config
        .sender
        .post_task(&TaskRequest {
            id: task_id,
            from: config.sender_id.clone(),
            to: vec![config.receiver_id.clone()],
            body: body.clone(),
            ttl: config.ttl.to_string(),
            failure_strategy: FailureStrategy::Discard,
            metadata: Value::String(config.run_id.clone()),
        })
        .await
        .context("posting task")?;
    let posted = Instant::now();

    let blocking = BlockingOptions {
        wait_time: Some(config.timeout),
        wait_count: Some(1),
    };
    let received_task = config
        .receiver
        .get_task::<BenchmarkBody>(&task_id, &blocking)
        .await
        .context("receiving task")?
        .ok_or_else(|| anyhow!("task was not delivered"))?;
    let received = Instant::now();
    verify_body(&received_task.body, config, sequence, "task")?;

    config
        .receiver
        .put_result(
            &TaskResult {
                from: config.receiver_id.clone(),
                to: vec![config.sender_id.clone()],
                task: task_id,
                status: WorkStatus::Succeeded,
                body: received_task.body,
                metadata: Value::String(config.run_id.clone()),
            },
            &task_id,
        )
        .await
        .context("submitting result")?;
    let answered = Instant::now();

    let result = config
        .sender
        .poll_results::<BenchmarkBody>(&task_id, &blocking)
        .await
        .context("receiving result")?
        .into_iter()
        .find(|result| result.status == WorkStatus::Succeeded)
        .ok_or_else(|| anyhow!("successful result was not delivered"))?;
    verify_body(&result.body, config, sequence, "result")?;
    let completed = Instant::now();

    Ok(Sample {
        post: posted.duration_since(started),
        delivery: received.duration_since(posted),
        answer: answered.duration_since(received),
        result_return: completed.duration_since(answered),
        total: completed.duration_since(started),
    })
}

fn verify_body(body: &BenchmarkBody, config: &Config, sequence: usize, kind: &str) -> Result<()> {
    if body.run_id != config.run_id
        || body.sequence != sequence
        || body.data.len() != config.payload.len()
    {
        return Err(anyhow!(
            "{kind} payload did not match the submitted payload"
        ));
    }
    Ok(())
}

fn draw_progress(done: usize, total: usize, samples: &[SampleResult], elapsed: Duration) {
    const WIDTH: usize = 30;
    const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let filled = done.saturating_mul(WIDTH) / total;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled));
    let errors = samples.iter().filter(|sample| sample.is_err()).count();
    let rate = done as f64 / elapsed.as_secs_f64().max(0.001);
    let frame = (elapsed.as_millis() / 100) as usize % SPINNER.len();
    print!(
        "\r\x1b[2K  \x1b[36m{}\x1b[0m [{}] {}/{}  {:>7.2} req/s  errors: {}",
        SPINNER[frame], bar, done, total, rate, errors
    );
    let _ = io::stdout().flush();
}

fn print_report(samples: &[SampleResult], elapsed: Duration, requested: usize, color: bool) {
    let successes: Vec<&Sample> = samples
        .iter()
        .filter_map(|sample| sample.as_ref().ok())
        .collect();
    let failures: Vec<&FailedSample> = samples
        .iter()
        .filter_map(|sample| sample.as_ref().err())
        .collect();
    let success_rate = successes.len() as f64 * 100.0 / requested as f64;
    let throughput = successes.len() as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
    let total: Vec<Duration> = successes.iter().map(|sample| sample.total).collect();

    println!("\n  Summary");
    println!("  ─────────────────────────────────────────────────────────");
    println!(
        "  Success rate       {}",
        colored_percent(success_rate, color)
    );
    println!("  Successful         {}", successes.len());
    println!("  Failed             {}", failures.len());
    println!("  Test duration      {}", format_duration(elapsed));
    println!("  Throughput         {:.2} task round trips/s", throughput);

    if !total.is_empty() {
        println!("\n  End-to-end latency");
        println!("  ─────────────────────────────────────────────────────────");
        println!(
            "  Fastest            {}",
            format_duration(*total.iter().min().unwrap())
        );
        println!("  Average            {}", format_duration(mean(&total)));
        println!(
            "  Slowest            {}",
            format_duration(*total.iter().max().unwrap())
        );
        println!(
            "  p50                {}",
            format_duration(percentile(&total, 0.50))
        );
        println!(
            "  p90                {}",
            format_duration(percentile(&total, 0.90))
        );
        println!(
            "  p95                {}",
            format_duration(percentile(&total, 0.95))
        );
        println!(
            "  p99                {}",
            format_duration(percentile(&total, 0.99))
        );

        let post: Vec<_> = successes.iter().map(|sample| sample.post).collect();
        let delivery: Vec<_> = successes.iter().map(|sample| sample.delivery).collect();
        let answer: Vec<_> = successes.iter().map(|sample| sample.answer).collect();
        let result_return: Vec<_> = successes
            .iter()
            .map(|sample| sample.result_return)
            .collect();
        println!("\n  Stage latency                 average          p95");
        println!("  ─────────────────────────────────────────────────────────");
        print_stage("Post task", &post);
        print_stage("Deliver to receiver", &delivery);
        print_stage("Submit result", &answer);
        print_stage("Return to sender", &result_return);
        print_latency_chart(&total, color);
    }

    if !failures.is_empty() {
        println!("\n  Errors");
        println!("  ─────────────────────────────────────────────────────────");
        for failure in failures.iter().take(5) {
            println!(
                "  #{} after {}: {}",
                failure.sequence,
                format_duration(failure.elapsed),
                failure.error
            );
        }
        if failures.len() > 5 {
            println!("  … and {} more", failures.len() - 5);
        }
    }
    println!();
}

fn print_stage(name: &str, values: &[Duration]) {
    println!(
        "  {:<24} {:>12} {:>12}",
        name,
        format_duration(mean(values)),
        format_duration(percentile(values, 0.95))
    );
}

fn print_latency_chart(values: &[Duration], color: bool) {
    let points = [0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 0.99, 1.00];
    let max = percentile(values, 1.0).as_secs_f64().max(f64::EPSILON);
    println!("\n  Latency distribution");
    println!("  ─────────────────────────────────────────────────────────");
    for point in points {
        let value = percentile(values, point);
        let width = ((value.as_secs_f64() / max) * 28.0).ceil() as usize;
        let bar = "■".repeat(width.max(1));
        if color {
            println!(
                "  {:>3.0}%  {:>10}  \x1b[36m{}\x1b[0m",
                point * 100.0,
                format_duration(value),
                bar
            );
        } else {
            println!(
                "  {:>3.0}%  {:>10}  {}",
                point * 100.0,
                format_duration(value),
                bar
            );
        }
    }
}

fn mean(values: &[Duration]) -> Duration {
    Duration::from_secs_f64(
        values.iter().map(Duration::as_secs_f64).sum::<f64>() / values.len() as f64,
    )
}

fn percentile(values: &[Duration], quantile: f64) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() as f64 * quantile).ceil().max(1.0) as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn format_duration(duration: Duration) -> String {
    let micros = duration.as_secs_f64() * 1_000_000.0;
    if micros < 1_000.0 {
        format!("{micros:.0} µs")
    } else if micros < 1_000_000.0 {
        format!("{:.2} ms", micros / 1_000.0)
    } else {
        format!("{:.2} s", micros / 1_000_000.0)
    }
}

fn colored_percent(percent: f64, color: bool) -> String {
    if !color {
        return format!("{percent:.2}%");
    }
    let code = if percent >= 100.0 {
        32
    } else if percent >= 95.0 {
        33
    } else {
        31
    };
    format!("\x1b[{code}m{percent:.2}%\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_nearest_rank_above_quantile() {
        let values: Vec<_> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(percentile(&values, 0.50), Duration::from_millis(50));
        assert_eq!(percentile(&values, 0.99), Duration::from_millis(99));
    }

    #[test]
    fn durations_choose_readable_units() {
        assert_eq!(format_duration(Duration::from_micros(42)), "42 µs");
        assert_eq!(format_duration(Duration::from_micros(1_500)), "1.50 ms");
        assert_eq!(format_duration(Duration::from_millis(1_500)), "1.50 s");
    }
}
