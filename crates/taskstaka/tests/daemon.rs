//! Drive the real router in-process with the real client.

use std::time::Duration;

use taskstaka::client::Client;
use taskstaka::server::Bound;
use taskstaka_core::command::AddTask;
use taskstaka_core::{Command, Event, Status, Store};

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

fn set(client: &Client, id: &str, status: Status) -> taskstaka_core::Outcome {
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
        .downcast_ref::<taskstaka::client::ApiError>()
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
    assert_eq!(next.items[0].tier, taskstaka_core::Tier::Resurfaced);

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
        !taskstaka::client::DaemonInfo::path(dir.path()).exists(),
        "daemon.json removed on exit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_context_filter_follows_the_root() {
    let (_dir, client, handle) = start().await;
    let personal = client
        .apply(&Command::Add(AddTask {
            title: "Bake a pie".into(),
            context: Some(taskstaka_core::Context::Personal),
            ..Default::default()
        }))
        .unwrap()
        .primary
        .unwrap();
    let apples = add(&client, "Buy apples", Some(personal.as_str()));
    let work = add(&client, "Ship the release", None);
    // Dragged under a work story: it is work now, like `plan` and `next` say.
    client
        .apply(&Command::Move {
            id: apples.parse().unwrap(),
            parent: Some(work.parse().unwrap()),
            after: None,
            before: None,
        })
        .unwrap();
    let ids = |ctx| -> Vec<String> {
        client
            .tasks_filtered(None, Some(ctx), false)
            .unwrap()
            .into_iter()
            .map(|v| v.task.id.to_string())
            .collect()
    };
    assert!(
        ids(taskstaka_core::Context::Work).contains(&apples),
        "follows its new root"
    );
    assert!(!ids(taskstaka_core::Context::Personal).contains(&apples));
    handle.abort();
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
        .find(|n| n.rule == taskstaka_core::nudge::Rule::AllBlocked)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn next_limit_zero_means_unlimited() {
    let (_dir, client, handle) = start().await;
    add(&client, "one", None);
    add(&client, "two", None);
    assert_eq!(
        client.next(None, Some(0)).unwrap().items.len(),
        2,
        "0 is not a limit"
    );
    assert_eq!(client.next(None, Some(1)).unwrap().items.len(), 1);
    assert_eq!(client.next(None, None).unwrap().items.len(), 2);
    client.shutdown().unwrap();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plate_endpoint_follows_the_hand_and_is_never_empty() {
    let (_dir, client, handle) = start().await;

    // Nothing ready at all: the plate is empty.
    assert!(client.plate(None).unwrap().is_none());

    let a = add(&client, "Renew the cert", None);
    let b = add(&client, "Write the memo", None);

    // Nothing started: up next is the first ready row in outline order.
    let p = client.plate(None).unwrap().expect("plate");
    assert!(!p.in_hand);
    assert_eq!(p.task.task.id.as_str(), a);

    // Starting a task takes it in hand.
    set(&client, &b, Status::Doing);
    let p = client.plate(None).unwrap().expect("plate");
    assert!(p.in_hand);
    assert_eq!(p.task.task.id.as_str(), b);
    assert_eq!(p.task.task.status, Status::Doing);

    // Put back: the plate offers another ready row, preferring never started.
    set(&client, &b, Status::Todo);
    let p = client.plate(None).unwrap().expect("plate");
    assert!(!p.in_hand);
    assert_eq!(p.task.task.id.as_str(), a);

    // Context filter applies.
    assert!(
        client
            .plate(Some(taskstaka_core::Context::Personal))
            .unwrap()
            .is_none()
    );

    handle.abort();
}
