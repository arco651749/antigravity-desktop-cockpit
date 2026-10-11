mod capture;
mod input;
mod server;

use server::StreamServer;
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=======================================================");
    println!(" Antigravity Desktop Cockpit - Stream Engine (Rust)    ");
    println!(" High-Performance Zero-Copy Windows Capture & Input    ");
    println!("=======================================================");

    let mut port = 48280;
    let args: Vec<String> = env::args().collect();
    for i in 1..args.len() {
        if (args[i] == "--port" || args[i] == "-p") && i + 1 < args.len() {
            if let Ok(parsed) = args[i + 1].parse::<u16>() {
                port = parsed;
            }
        }
    }

    let server = Arc::new(StreamServer::new());
    server.run(port).await?;

    Ok(())
}
