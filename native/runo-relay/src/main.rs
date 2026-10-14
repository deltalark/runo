//! `runo-relay [address]`: run the relay that lets Runo's phone app reach a computer from
//! anywhere. Put it behind HTTPS (Caddy, nginx or a tunnel) on a server you control.

fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("0.0.0.0:{}", runo_relay::PORT));
    let listener = match std::net::TcpListener::bind(&address) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("runo-relay: could not listen on {address}: {e}");
            std::process::exit(1);
        }
    };
    println!("Runo relay on {address}. Serve it over HTTPS, then run on your computer:");
    println!("  runo --phone --relay https://<this server>");
    runo_relay::Relay::new().serve(listener);
}
