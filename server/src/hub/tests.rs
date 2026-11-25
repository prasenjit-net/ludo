//! The timer, presence and room lifetime rules, driven with a made-up clock.

use super::*;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

const TIMING: Timing = Timing {
    turn: Duration::from_secs(10),
    away_turn: Duration::from_secs(15),
    idle_timeouts: 3,
    room_grace: Duration::from_secs(900),
    lobby_grace: Duration::from_secs(60),
};

struct World {
    hub: Hub,
    now: Instant,
}

impl World {
    fn new() -> Self {
        World {
            hub: Hub::new(TIMING),
            now: Instant::now(),
        }
    }

    fn connect(&self, conn: u64, client: &str) -> UnboundedReceiver<ServerMsg> {
        let (tx, rx) = unbounded_channel();
        self.hub.hello(
            conn,
            tx,
            client.into(),
            client.into(),
            Player::Red,
            self.now,
        );
        rx
    }

    /// Let time pass, ticking as the server does.
    fn run(&mut self, secs: u64) {
        for _ in 0..secs * 4 {
            self.now += Duration::from_millis(250);
            self.hub.tick(self.now);
        }
    }

    fn room<T>(&self, f: impl FnOnce(&Room) -> T) -> Option<T> {
        self.hub.lock().rooms.values().next().map(f)
    }

    /// A started game between "a" (connection 1) and "b" (connection 2).
    fn duel(&self) {
        self.connect(1, "a");
        self.connect(2, "b");
        self.hub.create_room(1, "duel", 2, self.now);
        self.hub.join_room(2, 1, self.now);
    }

    /// The seat of the person on this client id.
    fn seat(&self, client: &str) -> usize {
        self.room(|r| r.members.iter().position(|m| m.client_id == client))
            .flatten()
            .expect("seated")
    }

    fn turn(&self) -> usize {
        self.room(|r| r.game.as_ref().unwrap().turn).unwrap()
    }

    fn wait_left(&self) -> Duration {
        self.room(|r| r.deadline.unwrap() - self.now).unwrap()
    }
}

#[test]
fn an_offline_player_keeps_their_seat_however_long_they_are_gone() {
    let mut w = World::new();
    w.duel();
    let b = w.seat("b");
    w.hub.disconnect(2, w.now);
    w.run(200);
    assert!(w.room(|r| !r.game.as_ref().unwrap().left[b]).unwrap());

    let mut rx = w.connect(3, "b");
    assert!(matches!(
        rx.try_recv(),
        Ok(ServerMsg::State { you, .. }) if you == b
    ));
}

#[test]
fn a_connected_player_who_never_acts_goes_idle_and_is_not_removed() {
    let mut w = World::new();
    w.duel();
    w.run(200);
    let (idle, left) = w
        .room(|r| {
            let idle = r.members.iter().all(|m| m.idle && m.conn.is_some());
            (idle, r.game.as_ref().unwrap().left.clone())
        })
        .unwrap();
    assert!(idle);
    assert_eq!(left, [false, false]);
}

#[test]
fn acting_ends_idleness() {
    let w = World::new();
    w.duel();
    let turn = w.turn();
    let conn = {
        let mut inner = w.hub.lock();
        let room = inner.rooms.values_mut().next().unwrap();
        room.members[turn].idle = true;
        room.members[turn].timeouts = 3;
        room.members[turn].conn.unwrap()
    };
    w.hub.act(conn, w.now, |g| g.roll(3));
    assert!(
        w.room(|r| !r.members[turn].idle && r.members[turn].timeouts == 0)
            .unwrap()
    );
}

#[test]
fn dropping_on_your_turn_gives_the_longer_wait_and_returning_restores_it() {
    let w = World::new();
    w.duel();
    assert_eq!(w.wait_left(), TIMING.turn);
    let (client, conn) = w
        .room(|r| {
            let m = &r.members[r.game.as_ref().unwrap().turn];
            (m.client_id.clone(), m.conn.unwrap())
        })
        .unwrap();
    w.hub.disconnect(conn, w.now);
    assert_eq!(w.wait_left(), TIMING.away_turn);
    w.connect(3, &client);
    assert_eq!(w.wait_left(), TIMING.turn);
}

#[test]
fn a_flapping_connection_cannot_hold_up_the_turn() {
    let mut w = World::new();
    w.duel();
    let turn = w.turn();
    let (client, mut conn) = w
        .room(|r| {
            let m = &r.members[turn];
            (m.client_id.clone(), m.conn.unwrap())
        })
        .unwrap();
    let start = w.now;
    for next in 10..40 {
        w.hub.disconnect(conn, w.now);
        w.run(4);
        w.connect(next, &client);
        conn = next;
        if w.turn() != turn {
            break;
        }
        w.run(4);
        if w.turn() != turn {
            break;
        }
    }
    assert_ne!(w.turn(), turn);
    assert!(w.now - start < Duration::from_secs(45));
}

#[test]
fn a_game_with_nobody_connected_pauses_then_resumes() {
    let mut w = World::new();
    w.duel();
    w.hub.disconnect(1, w.now);
    w.hub.disconnect(2, w.now);
    let tokens = w.room(|r| r.game.clone()).unwrap();
    w.run(600);
    assert_eq!(w.room(|r| r.game.clone()).unwrap(), tokens);
    assert!(w.room(|r| r.deadline.is_none()).unwrap());

    w.connect(3, "a");
    assert!(
        w.room(|r| r.deadline.is_some() && r.empty_since.is_none())
            .unwrap()
    );
}

#[test]
fn an_abandoned_game_expires() {
    let mut w = World::new();
    w.duel();
    w.hub.disconnect(1, w.now);
    w.hub.disconnect(2, w.now);
    w.run(899);
    assert!(w.room(|_| ()).is_some());
    w.run(2);
    assert!(w.room(|_| ()).is_none());
}

#[test]
fn a_game_everyone_left_is_removed_at_once() {
    let w = World::new();
    w.connect(1, "a");
    w.hub.create_room(1, "solo", 4, w.now);
    w.hub.start(1, w.now);
    w.hub.leave(1, w.now);
    assert!(w.room(|_| ()).is_none());
}

#[test]
fn a_waiting_room_keeps_a_dropped_host_for_a_while() {
    let mut w = World::new();
    w.connect(1, "a");
    w.connect(2, "b");
    w.hub.create_room(1, "lobby", 4, w.now);
    w.hub.join_room(2, 1, w.now);
    w.hub.disconnect(1, w.now);
    w.run(30);
    assert!(w.room(|r| r.members.len() == 2 && r.host == "a").unwrap());

    w.connect(3, "a");
    w.run(120);
    assert!(w.room(|r| r.members.len() == 2 && r.host == "a").unwrap());

    w.hub.disconnect(3, w.now);
    w.run(61);
    assert!(w.room(|r| r.members.len() == 1 && r.host == "b").unwrap());
}

#[test]
fn an_empty_waiting_room_is_removed_after_the_grace_period() {
    let mut w = World::new();
    w.connect(1, "a");
    w.hub.create_room(1, "alone", 4, w.now);
    w.hub.disconnect(1, w.now);
    w.run(59);
    assert!(w.room(|_| ()).is_some());
    w.run(2);
    assert!(w.room(|_| ()).is_none());
}
