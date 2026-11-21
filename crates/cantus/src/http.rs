use reqwest::{
    Client, ClientBuilder,
    dns::{Addrs, Name, Resolve, Resolving},
};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
};
use tokio::net::lookup_host;

pub fn client_builder() -> ClientBuilder {
    Client::builder().dns_resolver(RouteResolver)
}

struct RouteResolver;

impl Resolve for RouteResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = lookup_host((name.as_str(), 0)).await?.filter(has_route).collect();
            if addresses.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NetworkUnreachable,
                    format!("No reachable address for {}", name.as_str()),
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn has_route(address: &SocketAddr) -> bool {
    let mut destination = *address;
    destination.set_port(443);
    // A UDP connect checks the kernel route without sending a packet.
    destination.is_ipv4() || UdpSocket::bind("[::]:0").and_then(|socket| socket.connect(destination)).is_ok()
}
