//! Daily Word: our five-letter word game. One puzzle per language per day, the same for everyone on the server;
//! six tries, hard mode, streaks, points and standings per space, results posted to the space's game channel and a
//! morning summary (by the "Daily Word" bot) that tags yesterday's players.
//!
//! The answer never leaves the server until you've finished, and every guess is checked here, so it can't be read
//! out of the app and results can't be faked. Answers come from our own curated lists (`words/`), shuffled with a
//! secret per-server seed, so the public lists don't give the order away.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use laklak_proto::word::{self, GuessReq, Mark, Overview, Player, Standing, Stats, Status, Today};
use laklak_proto::{Event, Id, MessageKind};
use serde::Deserialize;

use crate::auth::Auth;
use crate::{R, St, bad, conflict, db, messages, now_ms};

/// #1 is 2026-11-20 (days since 1970-01-01).
const DAY_ONE: i64 = 20777;
const DAY_MS: i64 = 86_400_000;
/// The morning summary goes out at this local hour.
const SUMMARY_HOUR: i64 = 9;

struct Lang {
    /// Answers in this server's secret order.
    answers: Vec<String>,
    allowed: HashSet<String>,
}

static LANGS: OnceLock<HashMap<&'static str, Lang>> = OnceLock::new();

fn lines(s: &str) -> impl Iterator<Item = String> + '_ {
    s.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string)
}

/// Load the word lists and shuffle the answers with this server's seed (made once, kept in `game_meta`).
pub async fn init(st: &St) -> anyhow::Result<()> {
    let seed: Option<String> = sqlx::query_scalar("SELECT value FROM game_meta WHERE key = 'seed'").fetch_optional(&st.db).await?;
    let seed: u64 = match seed.and_then(|s| s.parse().ok()) {
        Some(s) => s,
        None => {
            let s: u64 = rand::random();
            sqlx::query("INSERT OR REPLACE INTO game_meta (key, value) VALUES ('seed', ?)").bind(s.to_string()).execute(&st.db).await?;
            s
        }
    };
    let mut map = HashMap::new();
    for (code, answers, allowed, extra) in [
        ("en", include_str!("../words/en_answers.txt"), include_str!("../words/en_allowed.txt"), include_str!("../words/en_extra_allowed.txt")),
        ("tr", include_str!("../words/tr_answers.txt"), include_str!("../words/tr_allowed.txt"), include_str!("../words/tr_extra_allowed.txt")),
    ] {
        let mut answers: Vec<String> = lines(answers).collect();
        answers.sort();
        answers.dedup();
        let allowed: HashSet<String> = lines(allowed).chain(lines(extra)).chain(answers.iter().cloned()).collect();
        shuffle(&mut answers, seed ^ fnv(code));
        map.insert(code, Lang { answers, allowed });
    }
    let _ = LANGS.set(map);
    Ok(())
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

/// Fisher–Yates with splitmix64: the same seed always gives the same order.
fn shuffle(v: &mut [String], mut seed: u64) {
    let mut next = || {
        seed = seed.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    };
    for i in (1..v.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

fn lang(code: &str) -> R<&'static Lang> {
    LANGS.get().and_then(|m| m.get(code)).ok_or_else(|| bad("pick a language: en or tr"))
}

/// Today's day number in the game's time zone, and when tomorrow starts (ms).
fn today(st: &St) -> (i64, i64) {
    let offset = st.cfg.game_utc_offset_min * 60_000;
    let local = now_ms() + offset;
    let day = local.div_euclid(DAY_MS);
    (day, (day + 1) * DAY_MS - offset)
}

fn answer(l: &Lang, day: i64) -> &str {
    &l.answers[(day - DAY_ONE).rem_euclid(l.answers.len() as i64) as usize]
}

struct Play {
    guesses: Vec<String>,
    hard: bool,
    solved: bool,
    finished_at: Option<i64>,
}

async fn play(st: &St, user: Id, code: &str, day: i64) -> R<Option<Play>> {
    let row: Option<(String, bool, bool, Option<i64>)> =
        sqlx::query_as("SELECT guesses, hard, solved, finished_at FROM word_plays WHERE user_id = ? AND lang = ? AND day = ?")
            .bind(user)
            .bind(code)
            .bind(day)
            .fetch_optional(&st.db)
            .await?;
    Ok(row.map(|(g, hard, solved, finished_at)| Play { guesses: serde_json::from_str(&g).unwrap_or_default(), hard, solved, finished_at }))
}

fn status(p: Option<&Play>) -> Status {
    match p {
        Some(p) if p.solved => Status::Won,
        Some(p) if p.finished_at.is_some() => Status::Lost,
        _ => Status::Playing,
    }
}

fn marks(p: &Play, answer: &str) -> Vec<Vec<Mark>> {
    p.guesses.iter().map(|g| word::score(g, answer)).collect()
}

/// Points for one finished game: 6 for one try … 1 for six, 0 for a miss.
fn points(tries: usize, solved: bool) -> i64 {
    if solved { 7 - tries as i64 } else { 0 }
}

#[derive(Deserialize)]
pub struct WordQuery {
    pub lang: String,
    pub space_id: Option<Id>,
}

pub async fn get(State(st): State<St>, auth: Auth, Query(q): Query<WordQuery>) -> R<Json<Overview>> {
    Ok(Json(overview(&st, auth.user, &q.lang, q.space_id).await?))
}

pub async fn guess(State(st): State<St>, auth: Auth, Json(req): Json<GuessReq>) -> R<Json<Overview>> {
    let l = lang(&req.lang)?;
    let (day, _) = today(&st);
    let word = word::normalize(&req.lang, &req.word);
    if word.chars().count() != word::LENGTH {
        return Err(bad("five letters, please"));
    }
    if !l.allowed.contains(&word) {
        return Err(bad("not in the word list"));
    }
    let answer = answer(l, day);
    let mut p = play(&st, auth.user, &req.lang, day)
        .await?
        .unwrap_or(Play { guesses: vec![], hard: req.hard, solved: false, finished_at: None });
    if p.finished_at.is_some() {
        return Err(conflict("you've finished today's puzzle — a new one comes tomorrow"));
    }
    if p.hard {
        let earlier: Vec<word::Guess> = p.guesses.iter().map(|g| word::Guess { word: g.clone(), marks: word::score(g, answer) }).collect();
        if let Some(miss) = word::hard_mode_miss(&req.lang, &word, &earlier) {
            return Err(bad(format!("hard mode: {miss}")));
        }
    }
    p.guesses.push(word.clone());
    p.solved = word == answer;
    if p.solved || p.guesses.len() >= word::TRIES {
        p.finished_at = Some(now_ms());
    }
    sqlx::query(
        "INSERT INTO word_plays (user_id, lang, day, guesses, hard, solved, finished_at, space_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (user_id, lang, day) DO UPDATE SET guesses = excluded.guesses, solved = excluded.solved,
           finished_at = excluded.finished_at, space_id = COALESCE(excluded.space_id, word_plays.space_id)",
    )
    .bind(auth.user)
    .bind(&req.lang)
    .bind(day)
    .bind(serde_json::to_string(&p.guesses).unwrap_or_default())
    .bind(p.hard)
    .bind(p.solved)
    .bind(p.finished_at)
    .bind(req.space_id)
    .execute(&st.db)
    .await?;
    if p.finished_at.is_some()
        && let Some(space) = req.space_id
    {
        let _ = post_result(&st, auth.user, space, &req.lang, day, &p, answer).await;
    }
    Ok(Json(overview(&st, auth.user, &req.lang, req.space_id).await?))
}

fn flag(code: &str) -> &'static str {
    word::LANGS.iter().find(|(c, ..)| *c == code).map_or("", |(_, _, f)| f)
}

/// The spoiler-free result, as the player's own message in the space's game channel (if they may post there).
async fn post_result(st: &St, user: Id, space: Id, code: &str, day: i64, p: &Play, answer: &str) -> R<()> {
    let Some(channel) = game_channel(st, space).await? else { return Ok(()) };
    let acc = db::access(&st.db, user, channel).await?;
    if !acc.can_post() {
        return Ok(());
    }
    let tries = if p.solved { p.guesses.len().to_string() } else { "X".into() };
    let grid: Vec<String> = marks(p, answer)
        .iter()
        .map(|row| row.iter().map(|m| match m { Mark::Correct => "🟩", Mark::Present => "🟨", Mark::Absent => "⬛" }).collect())
        .collect();
    let hard = if p.hard { " · hard" } else { "" };
    let content = format!("Daily Word {} #{} · {tries}/6{hard}\n{}", flag(code), day - DAY_ONE + 1, grid.join("\n"));
    messages::post_system(st, &acc.ch, user, MessageKind::Default, None, &content).await?;
    Ok(())
}

async fn game_channel(st: &St, space: Id) -> R<Option<Id>> {
    Ok(sqlx::query_scalar::<_, Option<Id>>("SELECT game_channel_id FROM spaces WHERE id = ?")
        .bind(space)
        .fetch_optional(&st.db)
        .await?
        .flatten())
}

async fn overview(st: &St, user: Id, code: &str, space: Option<Id>) -> R<Overview> {
    let l = lang(code)?;
    let (day, next) = today(st);
    let answer = answer(l, day);
    let mine = play(st, user, code, day).await?;
    let state = status(mine.as_ref());
    let today = Today {
        lang: code.into(),
        number: day - DAY_ONE + 1,
        guesses: mine
            .as_ref()
            .map(|p| p.guesses.iter().map(|g| word::Guess { word: g.clone(), marks: word::score(g, answer) }).collect())
            .unwrap_or_default(),
        hard: mine.as_ref().is_some_and(|p| p.hard),
        status: state,
        answer: (state != Status::Playing).then(|| answer.to_string()),
        next_in_ms: next - now_ms(),
    };
    // The space's people: who played today, and the standings.
    let members: Vec<Id> = match space {
        Some(sp) if db::is_member(&st.db, sp, user).await? => db::space_members(&st.db, sp).await?,
        _ => vec![user],
    };
    let ids = db::json_ids(&members);
    let rows: Vec<(Id, String, i64, String, bool, Option<i64>)> = sqlx::query_as(
        "SELECT user_id, lang, day, guesses, solved, finished_at FROM word_plays
         WHERE user_id IN (SELECT value FROM json_each(?)) ORDER BY day",
    )
    .bind(&ids)
    .fetch_all(&st.db)
    .await?;
    let mut players = vec![];
    let (mut week, mut month, mut all): (HashMap<Id, Standing>, HashMap<Id, Standing>, HashMap<Id, Standing>) = Default::default();
    let mut stats = Stats { distribution: vec![0; word::TRIES], ..Default::default() };
    let mut streak_days: Vec<(i64, bool)> = vec![];
    for (uid, lc, d, guesses, solved, finished_at) in &rows {
        let g: Vec<String> = serde_json::from_str(guesses).unwrap_or_default();
        if lc == code && *d == day {
            let p = Play { guesses: g.clone(), hard: false, solved: *solved, finished_at: *finished_at };
            players.push(Player {
                user_id: *uid,
                tries: g.len() as u8,
                status: status(Some(&p)),
                marks: (state != Status::Playing && finished_at.is_some()).then(|| marks(&p, answer)),
            });
        }
        if finished_at.is_none() {
            continue;
        }
        let pts = points(g.len(), *solved);
        for (table, from) in [(&mut week, day - 6), (&mut month, day - 29), (&mut all, i64::MIN)] {
            if *d >= from {
                let e = table.entry(*uid).or_insert(Standing { user_id: *uid, points: 0, played: 0, won: 0 });
                e.points += pts;
                e.played += 1;
                e.won += u32::from(*solved);
            }
        }
        if *uid == user && lc == code {
            stats.played += 1;
            if *solved {
                stats.won += 1;
                if let Some(slot) = stats.distribution.get_mut(g.len().saturating_sub(1)) {
                    *slot += 1;
                }
            }
            streak_days.push((*d, *solved));
        }
    }
    // Streaks: consecutive days won; the current one may end today or yesterday.
    let (mut run, mut last) = (0u32, None::<i64>);
    for (d, solved) in &streak_days {
        run = if *solved && last == Some(d - 1) { run + 1 } else if *solved { 1 } else { 0 };
        last = solved.then_some(*d);
        stats.max_streak = stats.max_streak.max(run);
    }
    stats.current_streak = if last.is_some_and(|d| d >= day - 1) { run } else { 0 };
    players.sort_by_key(|p| (p.status != Status::Won, p.tries));
    let rank = |m: HashMap<Id, Standing>| {
        let mut v: Vec<Standing> = m.into_values().collect();
        v.sort_by_key(|s| (std::cmp::Reverse(s.points), std::cmp::Reverse(s.won), s.user_id));
        v
    };
    Ok(Overview { today, players, stats, week: rank(week), month: rank(month), all_time: rank(all) })
}

// ---------------------------------------------------------------- the morning summary

/// Every few minutes: once it's past the summary hour, post yesterday's results to each space's game channel.
pub async fn summaries(st: St) {
    loop {
        if let Err(e) = summarize(&st).await {
            tracing::warn!("daily word summary: {}", e.msg);
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

async fn summarize(st: &St) -> R<()> {
    let (day, _) = today(st);
    let local_hour = (now_ms() + st.cfg.game_utc_offset_min * 60_000).rem_euclid(DAY_MS) / 3_600_000;
    if local_hour < SUMMARY_HOUR {
        return Ok(());
    }
    let yesterday = day - 1;
    let spaces: Vec<(Id, Id)> = sqlx::query_as(
        "SELECT id, game_channel_id FROM spaces WHERE game_channel_id IS NOT NULL
         AND id NOT IN (SELECT space_id FROM game_summaries WHERE day = ?)",
    )
    .bind(yesterday)
    .fetch_all(&st.db)
    .await?;
    for (space, channel) in spaces {
        sqlx::query("INSERT OR IGNORE INTO game_summaries (space_id, day) VALUES (?, ?)").bind(space).bind(yesterday).execute(&st.db).await?;
        let members = db::space_members(&st.db, space).await?;
        let mut lines = vec![];
        for (code, _, flag) in word::LANGS {
            let rows: Vec<(Id, String, bool, i64)> = sqlx::query_as(
                "SELECT user_id, guesses, solved, finished_at FROM word_plays
                 WHERE lang = ? AND day = ? AND finished_at IS NOT NULL AND user_id IN (SELECT value FROM json_each(?))",
            )
            .bind(code)
            .bind(yesterday)
            .bind(db::json_ids(&members))
            .fetch_all(&st.db)
            .await?;
            if rows.is_empty() {
                continue;
            }
            let mut ranked: Vec<(Id, usize, bool, i64)> = rows
                .into_iter()
                .map(|(u, g, solved, at)| (u, serde_json::from_str::<Vec<String>>(&g).map_or(0, |g| g.len()), solved, at))
                .collect();
            ranked.sort_by_key(|(_, tries, solved, at)| (!solved, *tries, *at));
            let parts: Vec<String> = ranked
                .iter()
                .enumerate()
                .map(|(i, (u, tries, solved, _))| {
                    let crown = if i == 0 && *solved { "👑 " } else { "" };
                    let score = if *solved { format!("{tries}/6") } else { "X/6".into() };
                    format!("{crown}<@{u}> {score}")
                })
                .collect();
            lines.push(format!("{flag} {}", parts.join(" · ")));
        }
        if lines.is_empty() {
            continue;
        }
        let Ok(ch) = db::channel(&st.db, channel).await else { continue };
        let bot = bot_user(st, space).await?;
        let content = format!(
            "**Daily Word #{}** — yesterday's results\n{}\nToday's puzzles are up. Good luck!",
            yesterday - DAY_ONE + 1,
            lines.join("\n")
        );
        messages::post_system(st, &ch, bot, MessageKind::Default, None, &content).await?;
    }
    Ok(())
}

/// The Daily Word bot (made on first use, can't sign in) as a member of `space`.
async fn bot_user(st: &St, space: Id) -> R<Id> {
    let existing: Option<Id> = sqlx::query_scalar("SELECT id FROM users WHERE bot = 1 AND display_name = 'Daily Word'").fetch_optional(&st.db).await?;
    let id = match existing {
        Some(id) => id,
        None => {
            let id = st.id();
            let mut name = "daily.word".to_string();
            for n in 2..50 {
                let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username = ?)").bind(&name).fetch_one(&st.db).await?;
                if !taken {
                    break;
                }
                name = format!("daily.word{n}");
            }
            sqlx::query(
                "INSERT INTO users (id, username, display_name, password_hash, about, bot, created_at)
                 VALUES (?, ?, 'Daily Word', '!', 'Posts yesterday''s Daily Word results every morning.', 1, ?)",
            )
            .bind(id)
            .bind(&name)
            .bind(now_ms())
            .execute(&st.db)
            .await?;
            id
        }
    };
    let added = sqlx::query("INSERT OR IGNORE INTO members (space_id, user_id, role, joined_at) VALUES (?, ?, 'member', ?)")
        .bind(space)
        .bind(id)
        .bind(now_ms())
        .execute(&st.db)
        .await?;
    if added.rows_affected() > 0 {
        let s = db::space(&st.db, space).await?;
        if let (Some(member), Ok(user)) = (s.members.into_iter().find(|m| m.user_id == id), db::user(&st.db, id).await) {
            let all = db::space_members(&st.db, space).await?;
            st.hub.send(&all, &Event::MemberAdd { space_id: space, member, user });
        }
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_order() {
        let mut a: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        let mut b = a.clone();
        shuffle(&mut a, 42);
        shuffle(&mut b, 42);
        assert_eq!(a, b);
        let mut c = b.clone();
        shuffle(&mut c, 43);
        assert_ne!(b, c);
        assert_eq!(points(1, true), 6);
        assert_eq!(points(6, true), 1);
        assert_eq!(points(6, false), 0);
    }
}
