//! Which way a connection reaches the other device.

use std::{net::IpAddr, time::Duration};

use iroh::{
    TransportAddr,
    endpoint::transports::{Addr, PathSelection, PathSelectionContext, PathSelectionData, PathSelector},
};
use serde::{Deserialize, Serialize};

/// Keeps a same-route path from flapping over jitter.
const WORTHWHILE_GAIN: Duration = Duration::from_millis(5);

/// A direct path this quick runs through the local router or close to it, and
/// carries far more than a Bluetooth link can.
const QUICK_DIRECT: Duration = Duration::from_millis(30);

/// The ways to reach a device, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    /// Directly over the same local network.
    Local,
    /// Over a link between the two devices, such as Bluetooth.
    Nearby,
    /// Directly over the internet.
    Internet,
    /// Over the internet through a relay server, which only sees ciphertext.
    Relay,
}

impl Route {
    pub(crate) fn of(addr: &Addr) -> Self {
        match addr {
            Addr::Ip(addr) => Self::of_ip(addr.ip()),
            Addr::Custom(_) => Self::Nearby,
            Addr::Relay(..) => Self::Relay,
        }
    }

    pub(crate) fn of_transport(addr: &TransportAddr) -> Self {
        match addr {
            TransportAddr::Ip(addr) => Self::of_ip(addr.ip()),
            TransportAddr::Custom(_) => Self::Nearby,
            _ => Self::Relay,
        }
    }

    fn of_ip(ip: IpAddr) -> Self {
        let local = match ip.to_canonical() {
            IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
            IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
        };
        if local { Self::Local } else { Self::Internet }
    }

    /// Where a path stands against the others: this route's place, except that
    /// Bluetooth yields to a quick direct path, which it could not keep up with.
    fn standing(self, rtt: Duration) -> (Self, bool) {
        match self {
            Self::Internet if rtt < QUICK_DIRECT => (Self::Nearby, false),
            Self::Nearby => (Self::Nearby, true),
            other => (other, false),
        }
    }
}

/// Sends data over the best [`Route`], and the lowest latency within it.
#[derive(Debug)]
pub(crate) struct RoutePriority;

impl PathSelector for RoutePriority {
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let score = |path: &PathSelectionData<'_>| {
            let rtt = path.stats()?.rtt;
            Some((Route::of(&path.network_path().remote()).standing(rtt), rtt))
        };
        let current = context.current().and_then(|current| {
            context.paths().filter(|path| path.network_path() == current).filter_map(|p| score(&p)).min()
        });
        let best = context.paths().filter_map(|path| Some((score(&path)?, path))).min_by_key(|(score, _)| *score);

        let mut selection = PathSelection::none();
        if let Some(((standing, rtt), path)) = best {
            let better = current.is_none_or(|(current_standing, current_rtt)| {
                standing < current_standing || (standing == current_standing && rtt + WORTHWHILE_GAIN < current_rtt)
            });
            if better {
                selection.set(&path);
            }
        }
        selection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_rank_as_intended() {
        assert!(Route::Local < Route::Nearby);
        assert!(Route::Nearby < Route::Internet);
        assert!(Route::Internet < Route::Relay);
    }

    #[test]
    fn only_a_slow_direct_path_gives_way_to_bluetooth() {
        let bluetooth = Route::Nearby.standing(Duration::from_millis(40));
        assert!(Route::Internet.standing(Duration::from_millis(15)) < bluetooth);
        assert!(bluetooth < Route::Internet.standing(Duration::from_millis(60)));
        assert!(Route::Local.standing(Duration::from_millis(60)) < Route::Internet.standing(Duration::from_millis(5)));
    }

    #[test]
    fn addresses_are_classified() {
        for local in ["192.168.1.20", "10.0.0.4", "172.16.3.1", "169.254.1.1", "fe80::1", "fd12::1"] {
            assert_eq!(Route::of_ip(local.parse().unwrap()), Route::Local, "{local}");
        }
        for internet in ["8.8.8.8", "100.64.0.1", "2a00:1450::1"] {
            assert_eq!(Route::of_ip(internet.parse().unwrap()), Route::Internet, "{internet}");
        }
        assert_eq!(Route::of_ip("::ffff:192.168.1.20".parse().unwrap()), Route::Local);
    }
}
