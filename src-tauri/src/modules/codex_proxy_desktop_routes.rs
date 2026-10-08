//! Private, bounded connection mapping for the sidecar's asynchronous route observer.
//! The map follows TCP lifetimes, never the account's mutable current selection.
use super::*;
use crate::modules::codex_proxy_engine::RequestRouteObserver;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;

const PATH: &str = "/_cockpit/proxy-route";
const MAX_ROUTES: usize = 8192;
static SECRET: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());
type Key = (String, SocketAddr);
enum Route {
    Engine {
        source: SocketAddr,
        observer: Arc<RequestRouteObserver>,
    },
    Proxy(String),
    Direct,
}
impl Route {
    fn payload(&self) -> Value {
        match self {
            Self::Engine { source, observer } => {
                json!({"sourceIP": source.ip().to_string(), "sourcePort": source.port().to_string(), "observer": observer.as_ref()})
            }
            Self::Proxy(name) => json!({"route": {"kind": "proxy", "name": name}}),
            Self::Direct => json!({"route": {"kind": "direct"}}),
        }
    }
}
static CONNECTIONS: LazyLock<StdMutex<HashMap<Key, Arc<Route>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

pub(super) fn observer(proxy_url: &str) -> Option<RequestRouteObserver> {
    let url = url::Url::parse(proxy_url).ok()?;
    if url.host_str()? != "127.0.0.1" {
        return None;
    }
    Some(RequestRouteObserver {
        proxy_url: proxy_url.to_owned(),
        mapping_url: format!("http://127.0.0.1:{}{PATH}", url.port()?),
        controller_url: String::new(),
        controller_secret: SECRET.clone(),
        node_names: BTreeMap::new(),
        proxy_name: String::new(),
    })
}

pub(super) struct Guard {
    key: Key,
    value: Arc<Route>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut connections) = CONNECTIONS.lock() {
            if connections
                .get(&self.key)
                .is_some_and(|value| Arc::ptr_eq(value, &self.value))
            {
                connections.remove(&self.key);
            }
        }
    }
}

pub(super) fn register(
    account: &str,
    source: SocketAddr,
    upstream: Option<SocketAddr>,
    target: Option<&str>,
    lease: Option<&DesktopTunnel>,
) -> Option<Guard> {
    let value = if let (Some(upstream), Some(lease)) = (upstream, lease) {
        Route::Engine {
            source: upstream,
            observer: lease.request_route_observer(),
        }
    } else if let Some(target) = target {
        let url = url::Url::parse(target).ok()?;
        let name = format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str()?,
            url.port_or_known_default()?
        );
        Route::Proxy(name)
    } else {
        Route::Direct
    };
    let key = (account.to_owned(), source);
    let value = Arc::new(value);
    let mut connections = CONNECTIONS.lock().ok()?;
    if connections.len() >= MAX_ROUTES {
        return None;
    }
    connections.insert(key.clone(), value.clone());
    Some(Guard { key, value })
}

fn lookup(account: &str, target: &str, authorization: &str) -> Option<Value> {
    if authorization != format!("Bearer {}", *SECRET) {
        return None;
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != PATH {
        return None;
    }
    let query: HashMap<_, _> = url.query_pairs().collect();
    let ip: IpAddr = query.get("sourceIP")?.parse().ok()?;
    let port: u16 = query.get("sourcePort")?.parse().ok()?;
    if !ip.is_loopback() || port == 0 {
        return None;
    }
    let route = CONNECTIONS
        .lock()
        .ok()?
        .get(&(account.to_owned(), SocketAddr::new(ip, port)))
        .cloned()?;
    // Serialization happens only for observer reads, after releasing the map lock.
    Some(route.payload())
}

/// Only GET is intercepted. CONNECT/SOCKS negotiation remains byte-for-byte intact.
/// No controller I/O occurs here, and observer reads never increment business counters.
pub(super) async fn try_serve(client: &mut TcpStream, account: &str) -> Result<bool, String> {
    let mut first = [0u8];
    if client
        .peek(&mut first)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?
        != 1
        || first[0] != b'G'
    {
        return Ok(false);
    }
    let mut header = Vec::new();
    while header.len() < 4096 {
        header.push(client.read_u8().await.map_err(|_| "PROXY_CONNECT_FAILED")?);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let is_mapping_request = header.ends_with(b"\r\n\r\n")
        && std::str::from_utf8(&header)
            .ok()
            .and_then(|text| {
                let parts: Vec<_> = text.split("\r\n").next()?.split_whitespace().collect();
                Some(
                    parts.len() == 3
                        && parts[0] == "GET"
                        && parts[2] == "HTTP/1.1"
                        && parts[1].split('?').next() == Some(PATH),
                )
            })
            .unwrap_or(false);
    if !is_mapping_request {
        protocol::Protocol::HttpConnect
            .reply(client, protocol::Reply::BadRequest)
            .await?;
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let payload = std::str::from_utf8(&header).ok().and_then(|text| {
        if !header.ends_with(b"\r\n\r\n") {
            return None;
        }
        let mut lines = text.split("\r\n");
        let parts: Vec<_> = lines.next()?.split_whitespace().collect();
        if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
            return None;
        }
        let authorization = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))?
            .1
            .trim();
        lookup(account, parts[1], authorization)
    });
    let (status, body) = match payload {
        Some(value) => (
            "200 OK",
            serde_json::to_vec(&value).map_err(|_| "PROXY_CONNECT_FAILED")?,
        ),
        None => ("404 Not Found", Vec::new()),
    };
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    client
        .write_all(response.as_bytes())
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    client
        .write_all(&body)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_is_authenticated_scoped_and_removed_with_connection() {
        let source: SocketAddr = "127.0.0.1:41001".parse().unwrap();
        let account = "route-map-test";
        let guard = register(
            account,
            source,
            None,
            Some("http://user:secret@proxy.example:8080/private"),
            None,
        )
        .unwrap();
        let path = format!("{PATH}?sourceIP=127.0.0.1&sourcePort=41001");
        let auth = format!("Bearer {}", *SECRET);
        assert!(lookup(account, &path, "Bearer wrong").is_none());
        assert!(lookup("different-account", &path, &auth).is_none());
        let value = lookup(account, &path, &auth).unwrap();
        assert_eq!(value["route"]["name"], "http://proxy.example:8080");
        assert!(!value.to_string().contains("secret"));
        drop(guard);
        assert!(lookup(account, &path, &auth).is_none());
    }

    #[test]
    fn replaced_socket_mapping_cannot_be_removed_by_old_guard() {
        let source = "127.0.0.1:41002".parse().unwrap();
        let old = register("reused", source, None, None, None).unwrap();
        let current = register(
            "reused",
            source,
            None,
            Some("socks5://127.0.0.1:1000"),
            None,
        )
        .unwrap();
        drop(old);
        let path = format!("{PATH}?sourceIP=127.0.0.1&sourcePort=41002");
        assert_eq!(
            lookup("reused", &path, &format!("Bearer {}", *SECRET)).unwrap()["route"]["kind"],
            "proxy"
        );
        drop(current);
    }
    #[tokio::test]
    async fn authenticated_http_mapping_does_not_count_as_proxy_traffic() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let account = "http-route-map-test";
        let observation = entry::listening(account, port);
        let observed = observation.entry();
        let route = register(
            account,
            "127.0.0.1:41003".parse().unwrap(),
            None,
            None,
            None,
        )
        .unwrap();
        let task = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            super::super::handle(client, account, observed)
                .await
                .unwrap();
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET {PATH}?sourceIP=127.0.0.1&sourcePort=41003 HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n", *SECRET);
        client.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"direct\""));
        assert_eq!(entry::snapshot(account).unwrap().request_count, 0);
        task.await.unwrap();
        drop(route);
    }
}
