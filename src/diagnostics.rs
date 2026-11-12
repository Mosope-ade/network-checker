use serde::Deserialize;
use std::net::IpAddr;
use std::process::Command;

#[derive(Debug)]
pub struct LocalNetworkState {
    pub interface_name: Option<String>,
    pub interface_up: bool,
    pub local_ip: Option<String>,
    pub route_exists: bool,
    pub arp_resolved: bool,
}

#[derive(Debug)]
pub enum Diagnosis {
    InterfaceDown,
    NoRoute,
    GatewayNotResponding(String),
}

#[derive(Debug)]
pub struct DiagnosticReport {
    pub state: LocalNetworkState,
    pub diagnosis: Diagnosis,
}

#[derive(Deserialize)]
struct RouteEntry {
    dev: Option<String>,
    prefsrc: Option<String>,
}

#[derive(Deserialize)]
struct NeighEntry {
    state: Option<String>,
}

pub fn diagnose_gateway(gateway: IpAddr) -> DiagnosticReport {
    let mut state = LocalNetworkState {
        interface_name: None,
        interface_up: false,
        local_ip: None,
        route_exists: false,
        arp_resolved: false,
    };

    // 1. Check Route
    let route_output = Command::new("ip")
        .args(["-j", "route", "get", &gateway.to_string()])
        .output();

    if let Ok(output) = route_output {
        if output.status.success() {
            state.route_exists = true;
            if let Ok(routes) = serde_json::from_slice::<Vec<RouteEntry>>(&output.stdout) {
                if let Some(route) = routes.into_iter().next() {
                    state.interface_name = route.dev;
                    state.local_ip = route.prefsrc;
                }
            }
        }
    }

    // 2. Check Interface State
    if let Some(ref iface) = state.interface_name {
        let operstate_path = format!("/sys/class/net/{}/operstate", iface);
        if let Ok(content) = std::fs::read_to_string(operstate_path) {
            let s = content.trim();
            // Some virtual interfaces show "unknown" but are fully functional.
            state.interface_up = s == "up" || s == "unknown";
        }
    }

    // 3. Check ARP / Neighbour State
    let neigh_output = Command::new("ip")
        .args(["-j", "neigh", "show", &gateway.to_string()])
        .output();

    if let Ok(output) = neigh_output {
        if output.status.success() {
            if let Ok(neighs) = serde_json::from_slice::<Vec<NeighEntry>>(&output.stdout) {
                if let Some(neigh) = neighs.into_iter().next() {
                    if let Some(s) = neigh.state {
                        let s = s.to_uppercase();
                        if s != "INCOMPLETE" && s != "FAILED" {
                            state.arp_resolved = true;
                        }
                    }
                }
            }
        }
    }

    // 4. Formulate Diagnosis
    let diagnosis = if !state.route_exists {
        Diagnosis::NoRoute
    } else if !state.interface_up {
        Diagnosis::InterfaceDown
    } else if !state.arp_resolved {
        Diagnosis::GatewayNotResponding("ARP resolution failed".into())
    } else {
        Diagnosis::GatewayNotResponding("ICMP blocked or ignored".into())
    };

    DiagnosticReport { state, diagnosis }
}

impl std::fmt::Display for Diagnosis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InterfaceDown => f.write_str("Network interface is down"),
            Self::NoRoute => f.write_str("No route to gateway"),
            Self::GatewayNotResponding(reason) => write!(f, "Gateway not responding: {reason}"),
        }
    }
}
