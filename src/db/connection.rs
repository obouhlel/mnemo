//! Connexions Postgres : TLS optionnel, ouverture/migration et boucle LISTEN qui relaie
//! les événements `vault_events` vers l'interface et réveille le worker pour la synchro du coffre.

use super::*;

impl Worker {
    pub(super) async fn connect(&mut self, url: String) {
        self.emit(Msg::Connecting);
        if let Some(l) = self.listener.take() {
            l.abort();
        }
        self.client = None;
        self.last_try = Some(std::time::Instant::now());
        let client = match open_client(&url).await {
            Ok(c) => c,
            Err(e) => return self.emit(Msg::ConnectionFailed(pg_err(&e))),
        };
        // Migration, serialized across instances with an advisory lock
        let migrate = async {
            client.execute("SELECT pg_advisory_lock(424242)", &[]).await?;
            let r = client.batch_execute(SCHEMA).await;
            client.execute("SELECT pg_advisory_unlock(424242)", &[]).await?;
            r
        };
        if let Err(e) = migrate.await {
            return self.emit(Msg::ConnectionFailed(format!("Migration du schéma impossible : {}", pg_err(&e))));
        }
        self.client = Some(client);
        self.url = url.clone();
        let sink = EventSink {
            out: self.out.clone(),
            notify: self.notify.clone(),
            wake: self.wake.clone(),
            client_id: self.client_id.clone(),
        };
        self.listener = Some(tokio::spawn(listen_loop(url, sink)));
        self.emit(Msg::Connected);
    }
}

pub(super) fn wants_tls(url: &str) -> bool {
    let u = url.to_lowercase();
    ["sslmode=require", "sslmode=verify-ca", "sslmode=verify-full"].iter().any(|m| u.contains(m))
}

/// Le pilote ne connaît que `disable`, `prefer` et `require`. `verify-ca` et `verify-full` (courants dans les
/// adresses des bases hébergées) deviennent `require` : avec `tls()` ci-dessous, le certificat et le nom du
/// serveur sont de toute façon toujours vérifiés.
pub(super) fn pg_url(url: &str) -> String {
    let mut out = url.to_string();
    for mode in ["sslmode=verify-full", "sslmode=verify-ca"] {
        while let Some(i) = out.to_lowercase().find(mode) {
            out.replace_range(i..i + mode.len(), "sslmode=require");
        }
    }
    out
}

pub(super) fn tls() -> tokio_postgres_rustls::MakeRustlsConnect {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_postgres_rustls::MakeRustlsConnect::new(config)
}

/// Destination des NOTIFY : l'interface (`Msg::Event` + repaint) et la boucle du worker (`wake`), qui
/// compare aussitôt le dossier à la base quand une note, un dossier, une icône… change sur un autre appareil.
#[derive(Clone)]
pub(super) struct EventSink {
    pub(super) out: std::sync::mpsc::Sender<Msg>,
    pub(super) notify: Arc<dyn Fn() + Send + Sync>,
    pub(super) wake: mpsc::UnboundedSender<()>,
    pub(super) client_id: String,
}

impl EventSink {
    fn event(&self, ev: VaultEvent) {
        let mine = ev.updated_by.as_deref().is_some_and(|u| u.starts_with(self.client_id.as_str()));
        if ev.kind != "board" && !mine {
            let _ = self.wake.send(());
        }
        let _ = self.out.send(Msg::Event(ev));
        (self.notify)();
    }
}

pub(super) fn drive<S, T>(conn: Connection<S, T>, events: Option<EventSink>) -> tokio::task::JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut conn = conn;
        let mut stream = futures_util::stream::poll_fn(move |cx| conn.poll_message(cx));
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(AsyncMessage::Notification(n)) => {
                    if let Some(sink) = &events {
                        match serde_json::from_str::<VaultEvent>(n.payload()) {
                            Ok(ev) => sink.event(ev),
                            Err(e) => eprintln!("[db] bad notification payload: {e}"),
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[db] connection error: {e}");
                    break;
                }
            }
        }
    })
}

pub(super) async fn open_client(url: &str) -> Res<Client> {
    connect_with(url, None).await.map(|(c, _)| c)
}

pub(super) async fn connect_with(url: &str, events: Option<EventSink>) -> Res<(Client, tokio::task::JoinHandle<()>)> {
    let mut config: tokio_postgres::Config = pg_url(url).parse()?;
    config.connect_timeout(Duration::from_secs(10));
    if wants_tls(url) {
        let (c, conn) = config.connect(tls()).await?;
        Ok((c, drive(conn, events)))
    } else {
        let (c, conn) = config.connect(NoTls).await?;
        Ok((c, drive(conn, events)))
    }
}

/// Keeps a LISTEN connection alive forever, reconnecting with exponential backoff.
pub(super) async fn listen_loop(url: String, sink: EventSink) {
    let EventSink { out, notify, wake, .. } = sink.clone();
    let mut attempt = 0u32;
    let mut first = true;
    loop {
        match connect_with(&url, Some(sink.clone())).await {
            Ok((client, driver)) => match client.batch_execute("LISTEN vault_events").await {
                Ok(()) => {
                    attempt = 0;
                    let _ = out.send(Msg::Live(true));
                    if !first {
                        let _ = out.send(Msg::Resync);
                        let _ = wake.send(()); // la base a pu changer pendant la coupure
                    }
                    notify();
                    first = false;
                    let _ = driver.await; // returns when the connection drops
                    drop(client);
                }
                Err(e) => eprintln!("[db] LISTEN failed: {e}"),
            },
            Err(e) => eprintln!("[db] listener connection failed: {}", pg_err(&e)),
        }
        let _ = out.send(Msg::Live(false));
        notify();
        let delay = Duration::from_millis((500u64 << attempt.min(6)).min(30_000));
        attempt += 1;
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_modes_are_accepted() {
        let url = pg_url("postgres://u:p@h/db?sslmode=verify-full&application_name=x");
        assert_eq!(url, "postgres://u:p@h/db?sslmode=require&application_name=x");
        assert!(url.parse::<tokio_postgres::Config>().is_ok());
        assert!(wants_tls("postgres://h/db?sslmode=verify-full"));
        assert!(pg_url("host=h sslmode=VERIFY-CA").parse::<tokio_postgres::Config>().is_ok());
    }
}
