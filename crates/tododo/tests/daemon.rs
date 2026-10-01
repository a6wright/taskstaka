//! Drive the real router in-process with the real client.

use std::time::Duration;

use tododo::client::Client;
use tododo::server::Bound;
use tododo_core::command::AddTask;
use tododo_core::{Command, Event, Status, Store};

async fn start() -> (
    tempfile::TempDir,
    Client,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::init(dir.path()).unwrap());
    let bound = Bound::bind(dir.path(), 0).await.unwrap();
    let client = Client::at(&bound.addr.to_string());
    let handle = tokio::spawn(bound.serve());
    // Wait for the listener to actually accept.
    for _ in 0..50 {
        if client.health().is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (dir, client, handle)
}

fn add(client: &Client, title: &str, parent: Option<&str>) -> String {
    let out = client
        .apply(&Command::Add(AddTask {
            title: title.into(),
            parent: parent.map(|p| p.parse().unwrap()),
            ..Default::default()
        }))
        .unwrap();
    out.primary.unwrap().to_string()
}

fn set(client: &Client, id: &str, status: Status) -> tododo_core::Outcome {
    client
        .apply(&Command::SetStatus {
            id: id.parse().unwrap(),
            status,
            waiting_on: None,
            until: None,
            cascade: false,
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resurface_flow_over_http() {
    let (dir, client, handle) = start().await;

    let h = client.health().unwrap();
    assert_eq!(h.todo_dir, dir.path());
    let rev0 = h.rev;

    let story = add(&client, "Ship auth story", None);
    let a = add(&client, "Fix JWT", Some(&story));
    let b = add(&client, "Migration", Some(&story));

    let next = client.next(None, None).unwrap();
    assert_eq!(next.summary.blocked, 1);
    assert!(next.items.iter().all(|i| i.id.as_str() != story));
    assert!(next.rev > rev0);

    // Prefix resolution and the error envelope.
    assert_eq!(client.resolve(&story[..3]).unwrap().as_str(), story);
    let err = client.resolve("zzzzzz").unwrap_err();
    let api = err
        .downcast_ref::<tododo::client::ApiError>()
        .expect("typed api error");
    assert_eq!(api.status, 404);
    assert_eq!(api.code, "not_found");

    set(&client, &a, Status::Done);
    let out = set(&client, &b, Status::Done);
    assert!(
        out.events.contains(&Event::Resurfaced {
            id: story.parse().unwrap()
        }),
        "{out:?}"
    );

    let next = client.next(None, None).unwrap();
    assert_eq!(next.items[0].id.as_str(), story);
    assert_eq!(next.items[0].tier, tododo_core::Tier::Resurfaced);

    // Full snapshot rebuilds a tree with bodies and paths.
    let views = client.tasks(true).unwrap();
    assert_eq!(views.len(), 3);
    assert!(views.iter().all(|v| v.path.is_some()));
    let view = client.task(&story).unwrap();
    assert_eq!(view.children.len(), 2);
    assert!(view.derived.resurfaced_at.is_some());

    client.shutdown().unwrap();
    handle.await.unwrap().unwrap();
    assert!(
        !tododo::client::DaemonInfo::path(dir.path()).exists(),
        "daemon.json removed on exit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_poll_wakes_on_mutation() {
    let (_dir, client, handle) = start().await;
    let rev = client.health().unwrap().rev;

    let waiter = {
        let c = client.clone();
        tokio::task::spawn_blocking(move || c.changes(rev).unwrap())
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    add(&client, "wake up", None);
    let new_rev = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(new_rev > rev);

    client.shutdown().unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_daemon_on_same_dir_is_refused() {
    let (dir, client, handle) = start().await;
    let err = match Bound::bind(dir.path(), 0).await {
        Ok(_) => panic!("second bind should be refused"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("another daemon"), "{err}");
    client.shutdown().unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nudges_are_recomputed_after_mutations() {
    let (_dir, client, handle) = start().await;
    let id = add(&client, "Waiting on the world", None);
    client
        .apply(&Command::SetStatus {
            id: id.parse().unwrap(),
            status: Status::Waiting,
            waiting_on: Some("Bob".into()),
            until: None,
            cascade: false,
        })
        .unwrap();

    // The scheduler debounces mutations briefly, then republishes.
    let mut nudges = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        nudges = client.nudges().unwrap();
        if !nudges.is_empty() {
            break;
        }
    }
    let blocked = nudges
        .iter()
        .find(|n| n.rule == tododo_core::nudge::Rule::AllBlocked)
        .unwrap_or_else(|| panic!("{nudges:?}"));
    assert!(blocked.message.contains("waiting on Bob"));

    client.shutdown().unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_releases_long_polls_immediately() {
    let (_dir, client, handle) = start().await;
    let rev = client.health().unwrap().rev;
    let waiter = {
        let c = client.clone();
        tokio::task::spawn_blocking(move || c.changes(rev))
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    let t0 = std::time::Instant::now();
    client.shutdown().unwrap();
    // The long-poll must come back now, not after its 25 s deadline, and the
    // server must finish right behind it.
    let _ = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("long-poll released");
    tokio::time::timeout(Duration::from_secs(3), handle)
        .await
        .expect("server stopped")
        .unwrap()
        .unwrap();
    assert!(t0.elapsed() < Duration::from_secs(3));
}
