mod gui;

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
};

use anyhow::{Context, Result};
use chrono::{Local, Timelike};
use clap::{Args, Parser, Subcommand};
use cyber_runner::{
    build_http_text, build_packets, build_run_count_packet, default_run_start,
    http_client::{fetch_venues, send_packet_result, FetchVenuesOptions, DEFAULT_VENUE_PATH},
    load_venues,
    packets::{calculate_sign, rewrite_headers},
    serialize_body, ReplayConfig,
};
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct AppArgs {
    /// Run as command-line application. Without this flag and without a subcommand, GUI starts.
    #[arg(long)]
    cli: bool,

    #[command(flatten)]
    generate: GenerateArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Generate running/save HTTP packet files.
    Preview(GenerateArgs),
    /// Fetch venue JSON from a configured server.
    FetchVenues(FetchVenuesArgs),
    /// Print the running/run/count request.
    RunCount(RunCountArgs),
    /// Generate and optionally send packets to a non-production server.
    Replay(ReplayArgs),
}

#[derive(Args, Clone, Debug)]
struct GenerateArgs {
    #[arg(long, default_value = "CyberRunner/output/health_run_venues.json")]
    venues_json: PathBuf,
    #[arg(long, default_value_t = 0)]
    venue_index: usize,
    #[arg(long, default_value = "CyberRunner/output")]
    out_dir: PathBuf,
    #[arg(long, default_value_t = 10.0)]
    duration_minutes: f64,
    #[arg(long, default_value = "2.00")]
    result_km: String,
    #[arg(long, default_value_t = 2.0)]
    track_km: f64,
    #[arg(long, default_value = "")]
    login_name: String,
    #[arg(long, default_value = "121")]
    semester_id: String,
    #[arg(long, default_value = "2025-2026学年第2学期")]
    semester_name: String,
    #[arg(long, default_value = "0")]
    sex: String,
    #[arg(long, default_value = "0")]
    run_status: String,
    #[arg(long, default_value = "8.00")]
    standard_pace: String,
    #[arg(long, default_value_t = 15.0)]
    inset_m: f64,
    #[arg(long, default_value_t = 20260601)]
    seed: u64,
    #[arg(long)]
    authorization: Option<String>,
    #[arg(long, default_value = cyber_runner::models::DEFAULT_USER_AGENT)]
    user_agent: Option<String>,
    #[arg(long, default_value = cyber_runner::models::DEFAULT_REFERER)]
    referer: String,
}

#[derive(Args, Debug)]
struct FetchVenuesArgs {
    #[arg(long, default_value = cyber_runner::models::DEFAULT_BASE_URL)]
    base_url: String,
    #[arg(long)]
    authorization: Option<String>,
    #[arg(long, default_value = "CyberRunner/output/health_run_venues.json")]
    out: PathBuf,
    #[arg(long, default_value = DEFAULT_VENUE_PATH)]
    path: String,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
    #[arg(long)]
    longitude: Option<String>,
    #[arg(long)]
    latitude: Option<String>,
    #[arg(long)]
    open_type: Option<String>,
    #[arg(long)]
    token_query: bool,
}

#[derive(Args, Debug)]
struct RunCountArgs {
    #[command(flatten)]
    generate: GenerateArgs,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    send: bool,
}

#[derive(Args, Debug)]
struct ReplayArgs {
    #[command(flatten)]
    generate: GenerateArgs,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    send: bool,
}

fn main() -> Result<()> {
    let args = AppArgs::parse();
    match args.command {
        None if !args.cli => gui::run_gui(),
        None => preview(args.generate),
        Some(Command::Preview(generate)) => preview(generate),
        Some(Command::FetchVenues(args)) => {
            let count = fetch_venues(FetchVenuesOptions {
                base_url: &args.base_url,
                authorization: args.authorization.as_deref(),
                out: &args.out,
                path: &args.path,
                timeout_seconds: args.timeout,
                longitude: args.longitude.as_deref(),
                latitude: args.latitude.as_deref(),
                open_type: args.open_type.as_deref(),
                token_query: args.token_query,
            })?;
            println!("wrote: {}", args.out.display());
            println!("health running places: {count}");
            Ok(())
        }
        Some(Command::RunCount(args)) => run_count(args),
        Some(Command::Replay(args)) => replay(args),
    }
}

fn preview(args: GenerateArgs) -> Result<()> {
    let (venue_name, packets) = generate_packets(&args)?;
    let run_dir = write_packets(&args.out_dir, &packets)?;
    println!("wrote: {}", run_dir.display());
    println!("venue: {venue_name}");
    println!("packets: {}", packets.len());
    Ok(())
}

fn run_count(args: RunCountArgs) -> Result<()> {
    let config = build_config(&args.generate);
    let packet = build_run_count_packet(&config)?;
    let body = serialize_body(&packet.body, true)?;
    let headers = rewrite_headers(
        &packet.headers,
        &body,
        false,
        args.generate.authorization.as_deref(),
        args.generate.user_agent.as_deref(),
    );
    println!("{}", build_http_text(&packet, &headers, &body));
    if args.send {
        let base_url = args
            .base_url
            .as_deref()
            .context("--base-url is required when --send is set")?;
        let response = send_packet_result(base_url, &packet)?;
        println!("{}", response.log_text);
    }
    Ok(())
}

fn replay(args: ReplayArgs) -> Result<()> {
    let (venue_name, packets) = generate_packets(&args.generate)?;
    let run_dir = write_packets(&args.generate.out_dir, &packets)?;
    println!("wrote: {}", run_dir.display());
    println!("venue: {venue_name}");
    println!("packets: {}", packets.len());
    if args.send {
        let base_url = args
            .base_url
            .as_deref()
            .context("--base-url is required when --send is set")?;
        let mut uid = String::new();
        for mut packet in packets {
            wait_until_packet_time(&packet);
            if !uid.is_empty() {
                apply_uid_to_packet(
                    &mut packet,
                    &uid,
                    args.generate.authorization.as_deref(),
                    args.generate.user_agent.as_deref(),
                )?;
            }
            println!("sending packet {}/{}", packet.index, packet.total);
            let response = send_packet_result(base_url, &packet)?;
            println!("{}", response.log_text);
            if let Some(found_uid) = extract_uid_from_response(&response.body_text) {
                uid = found_uid;
                println!("captured uid for following packets: {uid}");
            }
        }
    }
    Ok(())
}

fn wait_until_packet_time(packet: &cyber_runner::GeneratedPacket) {
    let wait = packet
        .scheduled_at
        .signed_duration_since(Local::now().naive_local());
    if let Ok(wait) = wait.to_std() {
        if !wait.is_zero() {
            println!(
                "waiting {:.3}s until packet {}/{}...",
                wait.as_secs_f64(),
                packet.index,
                packet.total
            );
            thread::sleep(wait);
        }
    }
}

fn apply_uid_to_packet(
    packet: &mut cyber_runner::GeneratedPacket,
    uid: &str,
    authorization: Option<&str>,
    user_agent: Option<&str>,
) -> Result<()> {
    packet.body["uid"] = Value::String(uid.to_owned());
    let login_name = packet
        .body
        .get("loginName")
        .and_then(Value::as_str)
        .unwrap_or("");
    let timestamp = packet
        .body
        .get("timestamp")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("packet body timestamp is missing or not an integer"))?;
    packet.body["sign"] = Value::String(calculate_sign(uid, login_name, timestamp));
    let body = serialize_body(&packet.body, false)?;
    packet.headers = rewrite_headers(&packet.headers, &body, false, authorization, user_agent);
    Ok(())
}

fn extract_uid_from_response(response_body: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(response_body).ok()?;
    find_uid(&payload)
}

fn find_uid(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            if let Some(uid) = object.get("uid").and_then(Value::as_str) {
                if !uid.is_empty() {
                    return Some(uid.to_owned());
                }
            }
            object.values().find_map(find_uid)
        }
        Value::Array(items) => items.iter().find_map(find_uid),
        _ => None,
    }
}

fn generate_packets(args: &GenerateArgs) -> Result<(String, Vec<cyber_runner::GeneratedPacket>)> {
    let venues = load_venues(&args.venues_json)?;
    let venue = venues
        .iter()
        .find(|venue| venue.index == args.venue_index)
        .or_else(|| venues.get(args.venue_index))
        .with_context(|| format!("venue index {} not found", args.venue_index))?;
    let config = build_config(args);
    let packets = build_packets(venue, &config, "")?;
    Ok((venue.venue_name.clone(), packets))
}

fn build_config(args: &GenerateArgs) -> ReplayConfig {
    let now = Local::now()
        .naive_local()
        .with_nanosecond(0)
        .unwrap_or_else(|| Local::now().naive_local());
    ReplayConfig {
        login_name: args.login_name.trim().to_owned(),
        semester_id: args.semester_id.trim().to_owned(),
        semester_name: args.semester_name.trim().to_owned(),
        sex: args.sex.trim().to_owned(),
        run_status: args.run_status.trim().to_owned(),
        standard_pace: args.standard_pace.trim().to_owned(),
        result_km: args.result_km.trim().to_owned(),
        track_km: args.track_km,
        duration_minutes: args.duration_minutes,
        start_time: default_run_start(now, args.duration_minutes, 60),
        inset_m: args.inset_m,
        seed: args.seed,
        user_agent: args.user_agent.clone(),
        authorization: args.authorization.clone(),
        referer: args.referer.trim().to_owned(),
        packet_seconds: 60,
    }
}

fn write_packets(out_dir: &Path, packets: &[cyber_runner::GeneratedPacket]) -> Result<PathBuf> {
    fs::create_dir_all(out_dir)?;
    let run_dir = out_dir.join(format!(
        "replay_packets_{}",
        Local::now().format("%Y%m%d_%H%M%S")
    ));
    fs::create_dir_all(&run_dir)?;
    for packet in packets {
        let body = serialize_body(&packet.body, true)?;
        let text = build_http_text(packet, &packet.headers, &body);
        fs::write(
            run_dir.join(format!("{:03}_running_save.http", packet.index)),
            text + "\n",
        )?;
    }
    Ok(run_dir)
}
