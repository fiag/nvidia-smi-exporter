use anyhow::{Context, Result};
use clap::{App, Arg};
use lazy_static::lazy_static;
use log::*;
use prometheus::Encoder;
use std::process::Command;
use tide::log::LogMiddleware;
use tide::{http::mime, Body, Request, Response, Server, StatusCode};

lazy_static! {
    static ref METRIC_LIST: Vec<&'static str> = vec![
        "nvidia_fan_speed",
        "nvidia_temperature_gpu",
        "nvidia_clocks_gr",
        "nvidia_clocks_sm",
        "nvidia_clocks_mem",
        "nvidia_power_draw",
        "nvidia_utilization_gpu",
        "nvidia_utilization_memory",
        "nvidia_memory_total",
        "nvidia_memory_free",
        "nvidia_memory_used"
    ];
}

#[async_std::main]
async fn main() -> Result<()> {
    let matches = App::new("Nvidia SMI Exporter")
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .multiple(true)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::with_name("listen")
                .short("l")
                .long("listen")
                .takes_value(true)
                .help("Sets the level of verbosity"),
        )
        .get_matches();

    match matches.occurrences_of("verbose") {
        0 => tide::log::with_level(log::LevelFilter::Warn),
        1 => tide::log::with_level(log::LevelFilter::Info),
        2 => tide::log::with_level(log::LevelFilter::Debug),
        3 | _ => tide::log::with_level(log::LevelFilter::Trace),
    }

    let mut app = Server::new();

    app.with(LogMiddleware::new()); // 日志中间件
    app.with(tide_compress::CompressMiddleware::new()); // Outgoing compression middleware
    app.at("/").get(handle_home);
    app.at("/metrics").get(handle_metrics);

    let addr = matches.value_of("listen").unwrap_or_else(|| "0.0.0.0:9101");
    info!("Listen on {}", addr);
    app.listen(addr).await?;

    Ok(())
}

fn process_nvidia_smi() -> Result<String> {
    let output = Command::new("nvidia-smi")
        .arg("--query-gpu=name,index,fan.speed,temperature.gpu,clocks.gr,clocks.sm,clocks.mem,power.draw,utilization.gpu,utilization.memory,memory.total,memory.free,memory.used")
        .arg("--format=csv,noheader,nounits")
        .output()
        .with_context(|| "Failed to execute command")?;
    let stdout = output.stdout.as_slice();
    debug!("stdout: {}", String::from_utf8_lossy(stdout));
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(stdout);
    let mut buffer = String::new();
    for result in rdr.records() {
        let record = result?;
        debug!("{:?}", record);
        let name = record.get(0).unwrap();
        let index = record.get(1).unwrap().trim();
        for (idx, i) in (2..record.len()).enumerate() {
            let value = record.get(i).unwrap();
            buffer += &*format!(
                "{}{{gpu=\"{}\", name=\"{}\"}} {}\n",
                *METRIC_LIST.get(idx).unwrap(),
                index,
                name,
                value
            );
        }
    }

    Ok(buffer)
}

async fn handle_metrics(_req: Request<()>) -> tide::Result {
    let mut buffer = Vec::new();
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    match process_nvidia_smi() {
        Ok(nvidia_buffer) => {
            let mut buf: Vec<u8> = nvidia_buffer.as_bytes().iter().cloned().collect();
            buffer.append(&mut buf);
        }
        Err(e) => error!("Failed to process nvidia-smi, {}", e),
    }

    let response = Response::builder(StatusCode::Ok)
        .content_type(mime::PLAIN)
        .body(Body::from(buffer))
        .build();
    Ok(response)
}

async fn handle_home(_req: Request<()>) -> tide::Result {
    let body = "<html>
        <head><title>Nvidia SMI exporter</title></head>
        <body>
        <h1>Nvidia SMI exporter</h1>
        <p><a href='/metrics'>Metrics</a></p>
        </body>
        </html>";

    let body = Body::from(body);
    let res = Response::builder(StatusCode::Ok)
        .body(body)
        .content_type(mime::HTML)
        .build();
    Ok(res)
}
