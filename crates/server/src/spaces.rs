//! Spaces, channels, DMs, invites, members and bans.

use std::collections::{BTreeSet, HashMap, HashSet};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use natter_proto::{
    Ban, BanReq, Channel, ChannelKind, CreateChannel, CreateDm, CreateInvite, CreateRole, CreateSpace,
    CreateThread, EPOCH_MS, Event, Id, Invite, InvitePreview, MAX_NAME_LEN, MAX_TOPIC_LEN,
    MessageKind, Overwrite, OverwriteKind, Presence, ReorderRoles, RoleDef, SetOverwrite, Space, Status,
    UpdateChannel, UpdateMember, UpdateRole, UpdateSpace, channel_slug, perm,
};

use crate::auth::{Auth, clean, own_image_upload, release_upload};
use crate::{R, St, bad, conflict, db, forbidden, messages, not_found, now_ms, voice};

/// Your permissions and rank in `space`, or 403 unless you have every permission in `need`.
async fn require(st: &St, space: Id, user: Id, need: u64) -> R<(u64, i64)> {
    let (p, rank) = db::my_perms(&st.db, space, user).await?;
    if p & need != need {
        let name = perm::LIST.iter().find(|(f, ..)| need & f != 0 && p & f == 0).map_or("a", |(_, n, ..)| *n);
        return Err(forbidden(format!("that takes the {name} permission")));
    }
    Ok((p, rank))
}

/// `target`'s rank in `space` (404 when they aren't a member).
async fn rank_of(st: &St, space: Id, target: Id) -> R<i64> {
    Ok(db::perms(&st.db, space, target).await?.ok_or_else(|| not_found("member"))?.1)
}

/// Which channels each member can see.
type Sight = HashMap<Id, BTreeSet<Id>>;

fn sight(space: &Space) -> Sight {
    space
        .members
        .iter()
        .map(|m| {
            let seen = space.channels.iter().filter(|c| space.channel_permissions(m.user_id, c) != 0).map(|c| c.id).collect();
            (m.user_id, seen)
        })
        .collect()
}

async fn sight_now(st: &St, space: Id) -> R<Sight> {
    Ok(sight(&db::space(&st.db, space).await?))
}

/// After a permission change: everyone whose set of visible channels changed gets the space again as they now
/// see it (the app replaces it), so private channels appear and disappear at once.
async fn resync(st: &St, space: Id, before: &Sight) -> R<()> {
    let full = db::space(&st.db, space).await?;
    for (user, now) in sight(&full) {
        if before.get(&user) != Some(&now) {
            st.hub.send(&[user], &Event::SpaceCreate(db::visible_to(full.clone(), user)));
        }
    }
    Ok(())
}

/// Tell the space its roles changed.
async fn roles_changed(st: &St, space: Id) -> R<()> {
    let members = db::space_members(&st.db, space).await?;
    st.hub.send(
        &members,
        &Event::RolesUpdate {
            space_id: space,
            roles: db::roles(&st.db, space).await?,
            everyone_permissions: db::everyone_permissions(&st.db, space).await?,
        },
    );
    Ok(())
}

/// Tell the space about one member's new roles / nickname.
async fn member_changed(st: &St, space: Id, user: Id) -> R<()> {
    let s = db::space(&st.db, space).await?;
    if let Some(member) = s.members.into_iter().find(|m| m.user_id == user) {
        let all = db::space_members(&st.db, space).await?;
        st.hub.send(&all, &Event::MemberUpdate { space_id: space, member });
    }
    Ok(())
}

/// Hang up everyone in these channels and return the upload tokens of their attachments,
/// so the files can be removed once the rows are gone.
async fn purge_channels(st: &St, channels: &[Id]) -> R<Vec<String>> {
    let set: HashSet<Id> = channels.iter().copied().collect();
    for user in st.voice.users_in(&set) {
        voice::disconnect(st, user).await?;
    }
    db::attachment_tokens_in_channels(&st.db, channels).await
}

// ---------------------------------------------------------------- spaces

pub async fn create_space(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<CreateSpace>,
) -> R<Json<Space>> {
    let name = clean(&req.name, MAX_NAME_LEN, "space name")?;
    let (sid, now, me) = (st.id(), now_ms(), auth.user);
    let (text_cat, voice_cat, general, lounge, admin) = (st.id(), st.id(), st.id(), st.id(), st.id());
    let mut tx = st.db.begin().await?;
    sqlx::query("INSERT INTO spaces (id, name, owner_id, created_at, everyone_permissions) VALUES (?, ?, ?, ?, ?)")
        .bind(sid)
        .bind(&name)
        .bind(me)
        .bind(now)
        .bind(perm::EVERYONE_DEFAULT as i64)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO members (space_id, user_id, role, joined_at) VALUES (?, ?, 'owner', ?)",
    )
    .bind(sid)
    .bind(me)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    // Every space starts with an Admin role, so making someone an admin is one click.
    sqlx::query(
        "INSERT INTO roles (id, space_id, name, color, position, permissions, hoist, created_at)
         VALUES (?, ?, 'Admin', '#e5484d', 1, ?, 1, ?)",
    )
    .bind(admin)
    .bind(sid)
    .bind(perm::ADMINISTRATOR as i64)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    for (id, kind, name, pos, parent) in [
        (text_cat, "category", "Text Channels", 0, None),
        (voice_cat, "category", "Voice Channels", 1, None),
        (general, "text", "general", 0, Some(text_cat)),
        (lounge, "voice", "General", 0, Some(voice_cat)),
    ] {
        sqlx::query("INSERT INTO channels (id, space_id, kind, name, position, parent_id) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(sid)
            .bind(kind)
            .bind(name)
            .bind(pos)
            .bind(parent)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let space = db::space_with_users(&st.db, sid, me).await?;
    st.hub.send(&[me], &Event::SpaceCreate(space.clone()));
    Ok(Json(space))
}

pub async fn update_space(
    State(st): State<St>,
    auth: Auth,
    Path(id): Path<Id>,
    Json(req): Json<UpdateSpace>,
) -> R<StatusCode> {
    let me = auth.user;
    let (mine, rank) = db::my_perms(&st.db, id, me).await?;
    let members = db::space_members(&st.db, id).await?;
    if req.name.is_some() || req.icon.is_some() {
        require(&st, id, me, perm::MANAGE_SPACE).await?;
    }
    if let Some(everyone) = req.everyone_permissions {
        require(&st, id, me, perm::MANAGE_ROLES).await?;
        let before = sight_now(&st, id).await?;
        let old = db::everyone_permissions(&st.db, id).await?;
        let new = everyone & perm::ALL;
        if (old ^ new) & !mine != 0 {
            return Err(forbidden("you can only change permissions you have yourself"));
        }
        sqlx::query("UPDATE spaces SET everyone_permissions = ? WHERE id = ?")
            .bind(new as i64)
            .bind(id)
            .execute(&st.db)
            .await?;
        roles_changed(&st, id).await?;
        resync(&st, id, &before).await?;
    }
    if let Some(name) = &req.name {
        let name = clean(name, MAX_NAME_LEN, "space name")?;
        sqlx::query("UPDATE spaces SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(icon) = req.icon {
        let new = if icon == 0 {
            None
        } else {
            Some(own_image_upload(&st, me, icon).await?)
        };
        let old: Option<Id> = sqlx::query_scalar("SELECT icon_id FROM spaces WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await?;
        if old != new {
            release_upload(&st, old).await?;
            if let Some(n) = new {
                sqlx::query("UPDATE uploads SET used = 1 WHERE id = ?")
                    .bind(n)
                    .execute(&st.db)
                    .await?;
            }
            sqlx::query("UPDATE spaces SET icon_id = ? WHERE id = ?")
                .bind(new)
                .bind(id)
                .execute(&st.db)
                .await?;
        }
    }
    if let Some(new_owner) = req.owner_id {
        if rank != i64::MAX {
            return Err(forbidden("only the owner can hand the space over"));
        }
        if new_owner != me {
            if !db::is_member(&st.db, id, new_owner).await? {
                return Err(bad("the new owner must be a member"));
            }
            let mut tx = st.db.begin().await?;
            sqlx::query("UPDATE spaces SET owner_id = ? WHERE id = ?")
                .bind(new_owner)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE members SET role = 'owner' WHERE space_id = ? AND user_id = ?")
                .bind(id)
                .bind(new_owner)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE members SET role = 'member' WHERE space_id = ? AND user_id = ?")
                .bind(id)
                .bind(me)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }
    let space = db::space(&st.db, id).await?;
    if req.owner_id.is_some() {
        for m in space
            .members
            .iter()
            .filter(|m| m.user_id == me || Some(m.user_id) == req.owner_id)
        {
            st.hub.send(
                &members,
                &Event::MemberUpdate {
                    space_id: id,
                    member: m.clone(),
                },
            );
        }
    }
    st.hub.send(
        &members,
        &Event::SpaceUpdate {
            id,
            name: space.name,
            icon: space.icon,
            owner_id: space.owner_id,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_space(State(st): State<St>, auth: Auth, Path(id): Path<Id>) -> R<StatusCode> {
    if db::my_perms(&st.db, id, auth.user).await?.1 != i64::MAX {
        return Err(forbidden("only the owner can delete a space"));
    }
    let members = db::space_members(&st.db, id).await?;
    let channels: Vec<Id> = sqlx::query_scalar("SELECT id FROM channels WHERE space_id = ?")
        .bind(id)
        .fetch_all(&st.db)
        .await?;
    let tokens = purge_channels(&st, &channels).await?;
    let icon: Option<Id> = sqlx::query_scalar("SELECT icon_id FROM spaces WHERE id = ?")
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    release_upload(&st, icon).await?;
    sqlx::query("DELETE FROM spaces WHERE id = ?")
        .bind(id)
        .execute(&st.db)
        .await?;
    messages::delete_files(&st, &tokens);
    st.hub.send(&members, &Event::SpaceDelete { id });
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- invites

const INVITE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

type InviteRow = (String, Id, Id, i64, Option<i64>, Option<i64>, i64);

fn invite_from(r: InviteRow) -> Invite {
    Invite {
        code: r.0,
        space_id: r.1,
        creator_id: r.2,
        uses: r.3 as u32,
        max_uses: r.4.map(|v| v as u32),
        expires_at: r.5,
        created_at: r.6,
    }
}

pub async fn list_invites(
    State(st): State<St>,
    auth: Auth,
    Path(id): Path<Id>,
) -> R<Json<Vec<Invite>>> {
    let (p, _) = db::my_perms(&st.db, id, auth.user).await?;
    let rows: Vec<InviteRow> = sqlx::query_as(
        "SELECT code, space_id, creator_id, uses, max_uses, expires_at, created_at FROM invites
         WHERE space_id = ? AND (? OR creator_id = ?) ORDER BY created_at DESC",
    )
    .bind(id)
    .bind(p & perm::MANAGE_SPACE != 0)
    .bind(auth.user)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows.into_iter().map(invite_from).collect()))
}

pub async fn create_invite(
    State(st): State<St>,
    auth: Auth,
    Path(id): Path<Id>,
    Json(req): Json<CreateInvite>,
) -> R<Json<Invite>> {
    require(&st, id, auth.user, perm::CREATE_INVITES).await?;
    if req.max_uses.is_some_and(|m| !(1..=1000).contains(&m)) {
        return Err(bad("max uses must be 1 to 1000"));
    }
    if req
        .expires_in_secs
        .is_some_and(|s| !(60..=30 * 86_400).contains(&s))
    {
        return Err(bad("invites expire after 1 minute to 30 days, or never"));
    }
    let code: String = rand::random::<[u8; 8]>()
        .iter()
        .map(|b| INVITE_CHARS[*b as usize % INVITE_CHARS.len()] as char)
        .collect();
    let now = now_ms();
    let invite = Invite {
        code,
        space_id: id,
        creator_id: auth.user,
        uses: 0,
        max_uses: req.max_uses,
        expires_at: req.expires_in_secs.map(|s| now + s as i64 * 1000),
        created_at: now,
    };
    sqlx::query(
        "INSERT INTO invites (code, space_id, creator_id, max_uses, expires_at, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&invite.code)
    .bind(id)
    .bind(auth.user)
    .bind(invite.max_uses)
    .bind(invite.expires_at)
    .bind(now)
    .execute(&st.db)
    .await?;
    Ok(Json(invite))
}

pub async fn revoke_invite(
    State(st): State<St>,
    auth: Auth,
    Path(code): Path<String>,
) -> R<StatusCode> {
    let row: Option<(Id, Id)> =
        sqlx::query_as("SELECT space_id, creator_id FROM invites WHERE code = ?")
            .bind(&code)
            .fetch_optional(&st.db)
            .await?;
    let (space, creator) = row.ok_or_else(|| not_found("invite"))?;
    let (p, _) = db::my_perms(&st.db, space, auth.user).await?;
    if creator != auth.user && p & perm::MANAGE_SPACE == 0 {
        return Err(forbidden(
            "only its creator or someone with Manage space can revoke an invite",
        ));
    }
    sqlx::query("DELETE FROM invites WHERE code = ?")
        .bind(&code)
        .execute(&st.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The space a valid invite leads to.
pub async fn check_invite(st: &St, code: &str) -> R<Id> {
    let row: Option<(Id, i64, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT space_id, uses, max_uses, expires_at FROM invites WHERE code = ?")
            .bind(code)
            .fetch_optional(&st.db)
            .await?;
    match row {
        Some((space, uses, max, expires))
            if max.is_none_or(|m| uses < m) && expires.is_none_or(|e| e > now_ms()) =>
        {
            Ok(space)
        }
        _ => Err(not_found("invite (it may have expired)")),
    }
}

pub async fn invite_preview(
    State(st): State<St>,
    Path(code): Path<String>,
) -> R<Json<InvitePreview>> {
    let space = db::space(&st.db, check_invite(&st, &code).await?).await?;
    Ok(Json(InvitePreview {
        code,
        space_name: space.name,
        space_icon: space.icon,
        member_count: space.members.len() as u32,
        online_count: space
            .members
            .iter()
            .filter(|m| st.hub.presence(m.user_id) != Status::Offline)
            .count() as u32,
    }))
}

pub async fn join(State(st): State<St>, auth: Auth, Path(code): Path<String>) -> R<Json<Space>> {
    Ok(Json(join_space(&st, auth.user, &code).await?))
}

pub async fn join_space(st: &St, user: Id, code: &str) -> R<Space> {
    let space_id = check_invite(st, code).await?;
    if db::is_member(&st.db, space_id, user).await? {
        return db::space_with_users(&st.db, space_id, user).await;
    }
    let space = admit(st, user, space_id).await?;
    sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = ?")
        .bind(code)
        .execute(&st.db)
        .await?;
    Ok(space)
}

/// Add `user` to a space (invite or Nextcloud auto-join): unless banned, make them a member, catch them up,
/// introduce them to everyone and post "X joined".
pub async fn admit(st: &St, user: Id, space_id: Id) -> R<Space> {
    let banned: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bans WHERE space_id = ? AND user_id = ?)")
            .bind(space_id)
            .bind(user)
            .fetch_one(&st.db)
            .await?;
    if banned {
        return Err(forbidden("you are banned from this space"));
    }
    let added = sqlx::query("INSERT OR IGNORE INTO members (space_id, user_id, role, joined_at) VALUES (?, ?, 'member', ?)")
        .bind(space_id)
        .bind(user)
        .bind(now_ms())
        .execute(&st.db)
        .await?;
    if added.rows_affected() == 0 {
        return db::space_with_users(&st.db, space_id, user).await;
    }
    db::mark_space_read(&st.db, user, space_id).await?;

    let space = db::space_with_users(&st.db, space_id, user).await?;
    let others: Vec<Id> = space
        .members
        .iter()
        .map(|m| m.user_id)
        .filter(|&u| u != user)
        .collect();
    let member = space
        .members
        .iter()
        .find(|m| m.user_id == user)
        .cloned()
        .expect("just inserted");
    let me = db::user(&st.db, user).await?;
    st.hub.send(
        &others,
        &Event::MemberAdd {
            space_id,
            member,
            user: me,
        },
    );
    st.hub.send(
        &others,
        &Event::PresenceUpdate(Presence {
            user_id: user,
            status: st.hub.presence(user),
        }),
    );
    for &u in &others {
        st.hub.send(
            &[user],
            &Event::PresenceUpdate(Presence {
                user_id: u,
                status: st.hub.presence(u),
            }),
        );
    }
    st.hub.send(&[user], &Event::SpaceCreate(space.clone()));
    let channels: HashSet<Id> = space.channels.iter().map(|c| c.id).collect();
    for vs in st.voice.states_in(&channels) {
        st.hub.send(&[user], &Event::VoiceStateUpdate(vs));
    }
    if let Some(ch) = space
        .channels
        .iter()
        .filter(|c| c.kind == ChannelKind::Text)
        .min_by_key(|c| (c.position, c.id))
    {
        let msg = messages::post_system(st, ch, user, MessageKind::MemberJoin, None, "").await?;
        db::set_read(&st.db, user, ch.id, msg.id).await?;
    }
    db::space_with_users(&st.db, space_id, user).await
}

// ---------------------------------------------------------------- members & bans

pub async fn update_member(
    State(st): State<St>,
    auth: Auth,
    Path((space, target)): Path<(Id, Id)>,
    Json(req): Json<UpdateMember>,
) -> R<StatusCode> {
    let (mine, my_rank) = db::my_perms(&st.db, space, auth.user).await?;
    let their_rank = rank_of(&st, space, target).await?;
    if req.role.is_some() {
        return Err(bad("roles are given with PUT /api/spaces/{id}/members/{user}/roles/{role}; update Natter"));
    }
    if let Some(nick) = &req.nickname {
        if target != auth.user && !(mine & perm::MANAGE_NICKNAMES != 0 && my_rank > their_rank) {
            return Err(forbidden("you can't change their nickname"));
        }
        let nick = if nick.trim().is_empty() {
            None
        } else {
            Some(clean(nick, 32, "nickname")?)
        };
        sqlx::query("UPDATE members SET nickname = ? WHERE space_id = ? AND user_id = ?")
            .bind(nick)
            .bind(space)
            .bind(target)
            .execute(&st.db)
            .await?;
    }
    member_changed(&st, space, target).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take `user` out of `space`: hang up their calls there, drop read states, tell everyone.
async fn remove_from_space(st: &St, space: Id, user: Id) -> R<()> {
    let members = db::space_members(&st.db, space).await?;
    let channels: HashSet<Id> = sqlx::query_scalar("SELECT id FROM channels WHERE space_id = ?")
        .bind(space)
        .fetch_all(&st.db)
        .await?
        .into_iter()
        .collect();
    if st.voice.users_in(&channels).contains(&user) {
        voice::disconnect(st, user).await?;
    }
    sqlx::query("DELETE FROM members WHERE space_id = ? AND user_id = ?")
        .bind(space)
        .bind(user)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM channel_overwrites WHERE target_id = ? AND channel_id IN (SELECT id FROM channels WHERE space_id = ?)")
        .bind(user)
        .bind(space)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM read_states WHERE user_id = ? AND channel_id IN (SELECT id FROM channels WHERE space_id = ?)")
        .bind(user)
        .bind(space)
        .execute(&st.db)
        .await?;
    st.hub.send(
        &members,
        &Event::MemberRemove {
            space_id: space,
            user_id: user,
        },
    );
    st.hub.send(&[user], &Event::SpaceDelete { id: space });
    Ok(())
}

pub async fn remove_member(
    State(st): State<St>,
    auth: Auth,
    Path((space, target)): Path<(Id, Id)>,
) -> R<StatusCode> {
    let (mine, my_rank) = db::my_perms(&st.db, space, auth.user).await?;
    if target == auth.user {
        if my_rank == i64::MAX {
            return Err(bad("hand the space over or delete it before leaving"));
        }
    } else if mine & perm::KICK_MEMBERS == 0 || my_rank <= rank_of(&st, space, target).await? {
        return Err(forbidden("you can't kick them"));
    }
    remove_from_space(&st, space, target).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_bans(
    State(st): State<St>,
    auth: Auth,
    Path(space): Path<Id>,
) -> R<Json<Vec<Ban>>> {
    require(&st, space, auth.user, perm::BAN_MEMBERS).await?;
    let rows: Vec<(Id, String, i64)> = sqlx::query_as(
        "SELECT user_id, reason, created_at FROM bans WHERE space_id = ? ORDER BY created_at DESC",
    )
    .bind(space)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(user_id, reason, created_at)| Ban {
                user_id,
                reason,
                created_at,
            })
            .collect(),
    ))
}

pub async fn ban(
    State(st): State<St>,
    auth: Auth,
    Path((space, target)): Path<(Id, Id)>,
    Json(req): Json<BanReq>,
) -> R<StatusCode> {
    let (_, my_rank) = require(&st, space, auth.user, perm::BAN_MEMBERS).await?;
    if target == auth.user {
        return Err(bad("you can't ban yourself"));
    }
    db::user(&st.db, target).await?;
    let them = db::perms(&st.db, space, target).await?;
    if them.is_some_and(|(_, rank)| rank >= my_rank) {
        return Err(forbidden("you can't ban them"));
    }
    let reason: String = req.reason.trim().chars().take(512).collect();
    sqlx::query(
        "INSERT INTO bans (space_id, user_id, reason, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (space_id, user_id) DO UPDATE SET reason = excluded.reason",
    )
    .bind(space)
    .bind(target)
    .bind(reason)
    .bind(now_ms())
    .execute(&st.db)
    .await?;
    if them.is_some() {
        remove_from_space(&st, space, target).await?;
    }
    let secs = req.delete_message_secs.min(7 * 86_400) as i64;
    if secs > 0 {
        let since: Id = (now_ms() - secs * 1000 - EPOCH_MS).max(0) << 10;
        let rows: Vec<(Id, Id)> = sqlx::query_as(
            "SELECT m.channel_id, m.id FROM messages m JOIN channels c ON c.id = m.channel_id
             WHERE c.space_id = ? AND m.author_id = ? AND m.id >= ? ORDER BY m.channel_id",
        )
        .bind(space)
        .bind(target)
        .bind(since)
        .fetch_all(&st.db)
        .await?;
        let mut by_channel: Vec<(Id, Vec<Id>)> = vec![];
        for (ch, id) in rows {
            match by_channel.last_mut() {
                Some((c, ids)) if *c == ch => ids.push(id),
                _ => by_channel.push((ch, vec![id])),
            }
        }
        for (ch, ids) in by_channel {
            let ch = db::channel(&st.db, ch).await?;
            messages::purge(&st, &ch, &ids).await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unban(
    State(st): State<St>,
    auth: Auth,
    Path((space, target)): Path<(Id, Id)>,
) -> R<StatusCode> {
    require(&st, space, auth.user, perm::BAN_MEMBERS).await?;
    sqlx::query("DELETE FROM bans WHERE space_id = ? AND user_id = ?")
        .bind(space)
        .bind(target)
        .execute(&st.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- roles

fn role_color(c: &str) -> R<String> {
    let c = c.trim().to_ascii_lowercase();
    if c.is_empty() || (c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())) {
        Ok(c)
    } else {
        Err(bad("role colours look like #5d68ec"))
    }
}

fn role_emoji(e: &str) -> R<String> {
    let e = e.trim();
    if e.chars().count() > 8 || e.chars().any(char::is_whitespace) {
        return Err(bad("a role icon is one emoji"));
    }
    Ok(e.to_string())
}

/// A role of `space` that `me` (with permissions `mine`, rank `my_rank`) may change: Manage roles, and the role
/// sits below their own highest role.
async fn manageable_role(st: &St, space: Id, me: Id, role: Id) -> R<(RoleDef, u64, i64)> {
    let (mine, my_rank) = require(st, space, me, perm::MANAGE_ROLES).await?;
    let r = db::roles(&st.db, space)
        .await?
        .into_iter()
        .find(|r| r.id == role)
        .ok_or_else(|| not_found("role"))?;
    if r.position >= my_rank {
        return Err(forbidden("you can only manage roles below your own highest role"));
    }
    Ok((r, mine, my_rank))
}

pub async fn create_role(
    State(st): State<St>,
    auth: Auth,
    Path(space): Path<Id>,
    Json(req): Json<CreateRole>,
) -> R<Json<RoleDef>> {
    let (mine, _) = require(&st, space, auth.user, perm::MANAGE_ROLES).await?;
    let name = clean(&req.name, MAX_NAME_LEN, "role name")?;
    let permissions = req.permissions & perm::ALL;
    if permissions & !mine != 0 {
        return Err(forbidden("a new role can't have permissions you don't have"));
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE space_id = ?")
        .bind(space)
        .fetch_one(&st.db)
        .await?;
    if count >= 250 {
        return Err(bad("a space can have at most 250 roles"));
    }
    let role = RoleDef {
        id: st.id(),
        name,
        color: role_color(&req.color)?,
        emoji: role_emoji(&req.emoji)?,
        position: 1,
        permissions,
        hoist: req.hoist,
    };
    // New roles start at the bottom, just above @everyone.
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE roles SET position = position + 1 WHERE space_id = ?")
        .bind(space)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO roles (id, space_id, name, color, emoji, position, permissions, hoist, created_at)
         VALUES (?, ?, ?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(role.id)
    .bind(space)
    .bind(&role.name)
    .bind(&role.color)
    .bind(&role.emoji)
    .bind(role.permissions as i64)
    .bind(role.hoist)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    roles_changed(&st, space).await?;
    Ok(Json(role))
}

pub async fn update_role(
    State(st): State<St>,
    auth: Auth,
    Path((space, role)): Path<(Id, Id)>,
    Json(req): Json<UpdateRole>,
) -> R<StatusCode> {
    let (r, mine, _) = manageable_role(&st, space, auth.user, role).await?;
    let name = req.name.as_deref().map(|n| clean(n, MAX_NAME_LEN, "role name")).transpose()?;
    let color = req.color.as_deref().map(role_color).transpose()?;
    let emoji = req.emoji.as_deref().map(role_emoji).transpose()?;
    let permissions = req.permissions.map(|p| p & perm::ALL);
    if permissions.is_some_and(|p| (p ^ r.permissions) & !mine != 0) {
        return Err(forbidden("you can only change permissions you have yourself"));
    }
    let before = sight_now(&st, space).await?;
    sqlx::query(
        "UPDATE roles SET name = ?, color = ?, emoji = ?, permissions = ?, hoist = ? WHERE id = ? AND space_id = ?",
    )
    .bind(name.unwrap_or(r.name))
    .bind(color.unwrap_or(r.color))
    .bind(emoji.unwrap_or(r.emoji))
    .bind(permissions.unwrap_or(r.permissions) as i64)
    .bind(req.hoist.unwrap_or(r.hoist))
    .bind(role)
    .bind(space)
    .execute(&st.db)
    .await?;
    roles_changed(&st, space).await?;
    resync(&st, space, &before).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_role(
    State(st): State<St>,
    auth: Auth,
    Path((space, role)): Path<(Id, Id)>,
) -> R<StatusCode> {
    manageable_role(&st, space, auth.user, role).await?;
    let before = sight_now(&st, space).await?;
    // Members lose it through the foreign key; apps drop unknown role ids on RolesUpdate.
    sqlx::query("DELETE FROM roles WHERE id = ? AND space_id = ?")
        .bind(role)
        .bind(space)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM channel_overwrites WHERE target_id = ?")
        .bind(role)
        .execute(&st.db)
        .await?;
    roles_changed(&st, space).await?;
    resync(&st, space, &before).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Drag-to-reorder. Roles at or above your own highest role must keep their places at the top.
pub async fn reorder_roles(
    State(st): State<St>,
    auth: Auth,
    Path(space): Path<Id>,
    Json(req): Json<ReorderRoles>,
) -> R<StatusCode> {
    let (_, my_rank) = require(&st, space, auth.user, perm::MANAGE_ROLES).await?;
    let current = db::roles(&st.db, space).await?;
    let mut wanted = req.ids.clone();
    wanted.sort_unstable();
    let mut have: Vec<Id> = current.iter().map(|r| r.id).collect();
    have.sort_unstable();
    if wanted != have {
        return Err(bad("list every role of the space exactly once"));
    }
    let fixed: Vec<Id> = current.iter().take_while(|r| r.position >= my_rank).map(|r| r.id).collect();
    if req.ids[..fixed.len()] != fixed[..] {
        return Err(forbidden("you can only move roles below your own highest role"));
    }
    let n = req.ids.len() as i64;
    let mut tx = st.db.begin().await?;
    for (i, id) in req.ids.iter().enumerate() {
        sqlx::query("UPDATE roles SET position = ? WHERE id = ? AND space_id = ?")
            .bind(n - i as i64)
            .bind(id)
            .bind(space)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    roles_changed(&st, space).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_member_role(
    State(st): State<St>,
    auth: Auth,
    Path((space, target, role)): Path<(Id, Id, Id)>,
) -> R<StatusCode> {
    let (r, mine, _) = manageable_role(&st, space, auth.user, role).await?;
    if r.permissions & !mine != 0 {
        return Err(forbidden("you can't hand out a role with permissions you don't have"));
    }
    if !db::is_member(&st.db, space, target).await? {
        return Err(not_found("member"));
    }
    let before = sight_now(&st, space).await?;
    sqlx::query("INSERT OR IGNORE INTO member_roles (space_id, user_id, role_id) VALUES (?, ?, ?)")
        .bind(space)
        .bind(target)
        .bind(role)
        .execute(&st.db)
        .await?;
    member_changed(&st, space, target).await?;
    resync(&st, space, &before).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member_role(
    State(st): State<St>,
    auth: Auth,
    Path((space, target, role)): Path<(Id, Id, Id)>,
) -> R<StatusCode> {
    manageable_role(&st, space, auth.user, role).await?;
    let before = sight_now(&st, space).await?;
    sqlx::query("DELETE FROM member_roles WHERE space_id = ? AND user_id = ? AND role_id = ?")
        .bind(space)
        .bind(target)
        .bind(role)
        .execute(&st.db)
        .await?;
    member_changed(&st, space, target).await?;
    resync(&st, space, &before).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- channel permissions

/// Validate one override for a channel of `space`: the target is `@everyone` (the space id), one of its roles or a
/// member; only channel permissions; and nothing changes that `mine` doesn't include (`old` is the current one).
fn check_overwrite(
    space: &Space,
    target: Id,
    kind: Option<OverwriteKind>,
    allow: u64,
    deny: u64,
    old: Option<&Overwrite>,
    mine: u64,
) -> R<Overwrite> {
    let real = if target == space.id || space.roles.iter().any(|r| r.id == target) {
        OverwriteKind::Role
    } else if space.members.iter().any(|m| m.user_id == target) {
        OverwriteKind::Member
    } else {
        return Err(not_found("role or member"));
    };
    if kind.is_some_and(|k| k != real) {
        return Err(bad("that's not a role / member"));
    }
    let allow = allow & perm::CHANNEL;
    let deny = deny & perm::CHANNEL & !allow;
    let (old_allow, old_deny) = old.map_or((0, 0), |o| (o.allow, o.deny));
    if ((allow ^ old_allow) | (deny ^ old_deny)) & !mine != 0 {
        return Err(forbidden("you can only allow or deny permissions you have yourself"));
    }
    Ok(Overwrite { id: target, kind: real, allow, deny })
}

/// Set (or with allow = deny = 0, clear) a channel's override for a role, `@everyone` or a member.
pub async fn set_overwrite(
    State(st): State<St>,
    auth: Auth,
    Path((channel, target)): Path<(Id, Id)>,
    Json(req): Json<SetOverwrite>,
) -> R<StatusCode> {
    let acc = db::access(&st.db, auth.user, channel).await?;
    let Some(space) = acc.ch.space_id else {
        return Err(bad("direct messages have no permissions to set"));
    };
    if acc.ch.kind == ChannelKind::Thread {
        return Err(bad("threads follow their channel's permissions"));
    }
    if !acc.has(perm::MANAGE_ROLES) {
        return Err(forbidden("that takes the Manage roles permission"));
    }
    let full = db::space(&st.db, space).await?;
    let old = acc.ch.overwrites.iter().find(|o| o.id == target);
    let o = check_overwrite(&full, target, req.kind, req.allow, req.deny, old, acc.perms)?;
    let before = sight(&full);
    if o.allow == 0 && o.deny == 0 {
        sqlx::query("DELETE FROM channel_overwrites WHERE channel_id = ? AND target_id = ?")
            .bind(channel)
            .bind(target)
            .execute(&st.db)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO channel_overwrites (channel_id, target_id, kind, allow, deny) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (channel_id, target_id) DO UPDATE SET kind = excluded.kind, allow = excluded.allow, deny = excluded.deny",
        )
        .bind(channel)
        .bind(target)
        .bind(db::to_s(o.kind))
        .bind(o.allow as i64)
        .bind(o.deny as i64)
        .execute(&st.db)
        .await?;
    }
    let updated = db::channel(&st.db, channel).await?;
    st.hub.send(&db::audience(&st.db, &updated).await?, &Event::ChannelUpdate(updated));
    resync(&st, space, &before).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_overwrite(
    st: State<St>,
    auth: Auth,
    path: Path<(Id, Id)>,
) -> R<StatusCode> {
    set_overwrite(st, auth, path, Json(SetOverwrite::default())).await
}

// ---------------------------------------------------------------- channels

fn channel_name(kind: ChannelKind, name: &str) -> R<String> {
    if kind == ChannelKind::Text {
        let slug = channel_slug(name);
        if slug.is_empty() {
            return Err(bad("channel names need at least one letter or digit"));
        }
        Ok(slug)
    } else {
        clean(name, MAX_NAME_LEN, "channel name")
    }
}

fn check_topic(topic: &str) -> R<String> {
    let t = topic.trim();
    if t.chars().count() > MAX_TOPIC_LEN {
        return Err(bad(format!(
            "topic is longer than {MAX_TOPIC_LEN} characters"
        )));
    }
    Ok(t.to_string())
}

/// A category of the same space, for `parent_id`.
async fn check_parent(st: &St, space: Id, parent: Id) -> R<()> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM channels WHERE id = ? AND space_id = ? AND kind = 'category')",
    )
    .bind(parent)
    .bind(space)
    .fetch_one(&st.db)
    .await?;
    if !ok {
        return Err(bad("parent must be a category of this space"));
    }
    Ok(())
}

pub async fn create_channel(
    State(st): State<St>,
    auth: Auth,
    Path(space): Path<Id>,
    Json(req): Json<CreateChannel>,
) -> R<Json<Channel>> {
    let (mine, _) = require(&st, space, auth.user, perm::MANAGE_CHANNELS).await?;
    let full = db::space(&st.db, space).await?;
    let mut overwrites = vec![];
    for o in &req.overwrites {
        overwrites.push(check_overwrite(&full, o.id, Some(o.kind), o.allow, o.deny, None, mine)?);
    }
    if req.kind == ChannelKind::Thread {
        return Err(bad("start threads with POST /api/channels/{id}/threads"));
    }
    if req.kind.is_dm() {
        return Err(bad("use /api/dms for direct messages"));
    }
    let name = channel_name(req.kind, &req.name)?;
    let topic = check_topic(&req.topic)?;
    let parent = req.parent_id.filter(|&p| p != 0);
    if let Some(p) = parent {
        if req.kind == ChannelKind::Category {
            return Err(bad("categories can't be nested"));
        }
        check_parent(&st, space, p).await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE space_id = ?")
        .bind(space)
        .fetch_one(&st.db)
        .await?;
    if count >= 500 {
        return Err(conflict("a space can have at most 500 channels"));
    }
    let position: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE space_id = ? AND parent_id IS ?",
    )
    .bind(space)
    .bind(parent)
    .fetch_one(&st.db)
    .await?;
    let id = st.id();
    sqlx::query(
        "INSERT INTO channels (id, space_id, kind, name, topic, position, parent_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(space)
    .bind(db::to_s(req.kind))
    .bind(name)
    .bind(topic)
    .bind(position)
    .bind(parent)
    .execute(&st.db)
    .await?;
    for o in &overwrites {
        sqlx::query("INSERT INTO channel_overwrites (channel_id, target_id, kind, allow, deny) VALUES (?, ?, ?, ?, ?)")
            .bind(id)
            .bind(o.id)
            .bind(db::to_s(o.kind))
            .bind(o.allow as i64)
            .bind(o.deny as i64)
            .execute(&st.db)
            .await?;
    }
    let ch = db::channel(&st.db, id).await?;
    st.hub.send(&db::audience(&st.db, &ch).await?, &Event::ChannelCreate(ch.clone()));
    Ok(Json(ch))
}

pub async fn update_channel(
    State(st): State<St>,
    auth: Auth,
    Path(id): Path<Id>,
    Json(req): Json<UpdateChannel>,
) -> R<Json<Channel>> {
    let acc = db::access(&st.db, auth.user, id).await?;
    let ch = &acc.ch;
    let before = match (ch.space_id, req.parent_id) {
        (Some(sp), Some(_)) => Some((sp, sight_now(&st, sp).await?)),
        _ => None,
    };
    match ch.kind {
        ChannelKind::Dm => return Err(bad("direct messages can't be edited")),
        ChannelKind::GroupDm => {
            if req.position.is_some() || req.parent_id.is_some() || req.read_only.is_some() {
                return Err(bad("group DMs only have a name and a topic"));
            }
        }
        ChannelKind::Thread => {
            if !acc.has(perm::MANAGE_THREADS) && ch.thread.as_ref().map(|t| t.owner_id) != Some(auth.user) {
                return Err(forbidden("only its starter or someone with Manage threads can edit a thread"));
            }
            if req.topic.is_some()
                || req.position.is_some()
                || req.parent_id.is_some()
                || req.read_only.is_some()
            {
                return Err(bad("threads only have a name and an archived flag"));
            }
        }
        _ if !acc.has(perm::MANAGE_CHANNELS) => return Err(forbidden("that takes the Manage channels permission")),
        _ => {}
    }
    if let Some(archived) = req.archived {
        if ch.kind != ChannelKind::Thread {
            return Err(bad("only threads can be archived"));
        }
        sqlx::query("UPDATE channels SET archived = ? WHERE id = ?")
            .bind(archived)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(name) = &req.name {
        let name = if ch.kind == ChannelKind::GroupDm && name.trim().is_empty() {
            String::new()
        } else {
            channel_name(ch.kind, name)?
        };
        sqlx::query("UPDATE channels SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(topic) = &req.topic {
        sqlx::query("UPDATE channels SET topic = ? WHERE id = ?")
            .bind(check_topic(topic)?)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(pos) = req.position {
        if !(0..=10_000).contains(&pos) {
            return Err(bad("position must be 0 to 10000"));
        }
        sqlx::query("UPDATE channels SET position = ? WHERE id = ?")
            .bind(pos)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(parent) = req.parent_id {
        let parent = (parent != 0).then_some(parent);
        if let Some(p) = parent {
            if ch.kind == ChannelKind::Category {
                return Err(bad("categories can't be nested"));
            }
            check_parent(&st, ch.space_id.unwrap_or_default(), p).await?;
        }
        sqlx::query("UPDATE channels SET parent_id = ? WHERE id = ?")
            .bind(parent)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    if let Some(ro) = req.read_only {
        sqlx::query("UPDATE channels SET read_only = ? WHERE id = ?")
            .bind(ro)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    let updated = db::channel(&st.db, id).await?;
    st.hub.send(
        &db::audience(&st.db, &updated).await?,
        &Event::ChannelUpdate(updated.clone()),
    );
    if let Some((sp, before)) = before {
        resync(&st, sp, &before).await?;
    }
    if updated.kind == ChannelKind::Thread {
        starter_update(&st, &updated).await?;
    }
    Ok(Json(updated))
}

pub async fn delete_channel(State(st): State<St>, auth: Auth, Path(id): Path<Id>) -> R<StatusCode> {
    let acc = db::access(&st.db, auth.user, id).await?;
    let Some(space) = acc.ch.space_id else {
        return Err(bad("direct messages can't be deleted"));
    };
    let own_thread = acc
        .ch
        .thread
        .as_ref()
        .is_some_and(|t| t.owner_id == auth.user);
    let allowed = if acc.ch.kind == ChannelKind::Thread {
        own_thread || acc.has(perm::MANAGE_THREADS)
    } else {
        acc.has(perm::MANAGE_CHANNELS)
    };
    if !allowed {
        return Err(forbidden("you can't delete that channel"));
    }
    // Only those who could see it hear that it's gone.
    let members = db::audience(&st.db, &acc.ch).await?;
    // A text channel takes its threads with it.
    let mut gone = vec![id];
    if acc.ch.kind == ChannelKind::Text {
        let threads: Vec<Id> =
            sqlx::query_scalar("SELECT id FROM channels WHERE parent_id = ? AND kind = 'thread'")
                .bind(id)
                .fetch_all(&st.db)
                .await?;
        gone.extend(threads);
    }
    if acc.ch.kind == ChannelKind::Category {
        let children: Vec<Id> = sqlx::query_scalar(
            "UPDATE channels SET parent_id = NULL WHERE parent_id = ? RETURNING id",
        )
        .bind(id)
        .fetch_all(&st.db)
        .await?;
        for c in children {
            st.hub.send(
                &members,
                &Event::ChannelUpdate(db::channel(&st.db, c).await?),
            );
        }
    }
    let tokens = purge_channels(&st, &gone).await?;
    sqlx::query("DELETE FROM channels WHERE id IN (SELECT value FROM json_each(?))")
        .bind(db::json_ids(&gone))
        .execute(&st.db)
        .await?;
    messages::delete_files(&st, &tokens);
    for &c in gone.iter().rev() {
        st.hub.send(
            &members,
            &Event::ChannelDelete {
                id: c,
                space_id: Some(space),
            },
        );
    }
    if acc.ch.kind == ChannelKind::Thread {
        starter_update(&st, &acc.ch).await?; // its preview disappears
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- threads

/// Re-send a thread's starter message so its preview (name, count, archived) refreshes everywhere.
async fn starter_update(st: &St, thread: &Channel) -> R<()> {
    let (Some(parent), Some(space), Some(info)) =
        (thread.parent_id, thread.space_id, &thread.thread)
    else {
        return Ok(());
    };
    let _ = space;
    if let Ok(m) = db::message(&st.db, parent, info.starter_id).await {
        let parent = db::channel(&st.db, parent).await?;
        st.hub.send(&db::audience(&st.db, &parent).await?, &Event::MessageUpdate(m));
    }
    Ok(())
}

/// The plain start of a message (mentions and markdown marks dropped), for a thread's default name.
fn thread_name_from(content: &str) -> String {
    let mut text = String::new();
    let mut rest = content;
    while let Some(i) = rest.find('<') {
        text.push_str(&rest[..i]);
        let tail = &rest[i..];
        let mention_end = tail.find('>').filter(|&end| {
            let inner = &tail[1..end];
            inner.len() > 1
                && (inner.starts_with('@') || inner.starts_with('#'))
                && inner[1..].bytes().all(|b| b.is_ascii_digit())
        });
        match mention_end {
            Some(end) => rest = &tail[end + 1..],
            None => {
                text.push('<');
                rest = &tail[1..];
            }
        }
    }
    text.push_str(rest);
    let plain: String = text.chars().filter(|c| !"*_~`|>#".contains(*c)).collect();
    let name: String = plain
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(40)
        .collect();
    if name.is_empty() {
        "Thread".into()
    } else {
        name
    }
}

pub async fn create_thread(
    State(st): State<St>,
    auth: Auth,
    Path(parent): Path<Id>,
    Json(req): Json<CreateThread>,
) -> R<Json<Channel>> {
    let me = auth.user;
    let acc = db::access(&st.db, me, parent).await?;
    let Some(space) = acc.ch.space_id.filter(|_| acc.ch.kind == ChannelKind::Text) else {
        return Err(bad("threads start in text channels"));
    };
    if !acc.has(perm::CREATE_THREADS) {
        return Err(forbidden("that takes the Create threads permission"));
    }
    let custom = (!req.name.trim().is_empty())
        .then(|| clean(&req.name, MAX_NAME_LEN, "thread name"))
        .transpose()?;
    let (starter, name) = match req.message_id {
        Some(mid) => {
            let msg = db::message(&st.db, parent, mid).await?;
            if msg.kind == MessageKind::ThreadCreated {
                return Err(bad("that message can't start a thread"));
            }
            if msg.thread.is_some() {
                return Err(conflict("that message already has a thread"));
            }
            (
                mid,
                custom.unwrap_or_else(|| thread_name_from(&msg.content)),
            )
        }
        None => {
            let name = custom.ok_or_else(|| bad("name the thread"))?;
            if !acc.can_post() {
                return Err(forbidden("you can't post in this channel"));
            }
            let msg =
                messages::post_system(&st, &acc.ch, me, MessageKind::ThreadCreated, None, &name)
                    .await?;
            (msg.id, name)
        }
    };
    let id = st.id();
    let res = sqlx::query(
        "INSERT INTO channels (id, space_id, kind, name, parent_id, starter_id, owner_id)
         VALUES (?, ?, 'thread', ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(space)
    .bind(&name)
    .bind(parent)
    .bind(starter)
    .bind(me)
    .execute(&st.db)
    .await;
    match res {
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(conflict("that message already has a thread"));
        }
        r => r?,
    };
    db::set_read(&st.db, me, id, 0).await?;
    let thread = db::channel(&st.db, id).await?;
    let _ = space;
    st.hub.send(&db::audience(&st.db, &thread).await?, &Event::ChannelCreate(thread.clone()));
    starter_update(&st, &thread).await?;
    Ok(Json(thread))
}

pub async fn list_threads(
    State(st): State<St>,
    auth: Auth,
    Path(parent): Path<Id>,
) -> R<Json<Vec<Channel>>> {
    let acc = db::access(&st.db, auth.user, parent).await?;
    if acc.ch.kind != ChannelKind::Text {
        return Err(bad("only text channels have threads"));
    }
    let rows = sqlx::query_as::<_, db::ChannelRow>(db::channel_select!(
        "WHERE parent_id = ? AND kind = 'thread' ORDER BY COALESCE(last_message_id, id) DESC"
    ))
    .bind(parent)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(
        rows.into_iter().map(|r| r.into_channel(vec![])).collect(),
    ))
}

/// Posting in an archived thread brings it back.
pub async fn unarchive_on_post(st: &St, thread: &Channel) -> R<()> {
    if thread.thread.as_ref().is_some_and(|t| t.archived) {
        sqlx::query("UPDATE channels SET archived = 0 WHERE id = ?")
            .bind(thread.id)
            .execute(&st.db)
            .await?;
        let fresh = db::channel(&st.db, thread.id).await?;
        st.hub.send(
            &db::audience(&st.db, &fresh).await?,
            &Event::ChannelUpdate(fresh.clone()),
        );
        starter_update(st, &fresh).await?;
    }
    Ok(())
}

/// Hourly: archive threads that have been quiet for a week.
pub async fn archive_idle_threads(st: St) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        every.tick().await;
        let cutoff: Id = (now_ms() - 7 * 86_400_000 - EPOCH_MS).max(0) << 10;
        let res: Result<Vec<Id>, _> = sqlx::query_scalar(
            "UPDATE channels SET archived = 1
             WHERE kind = 'thread' AND archived = 0 AND MAX(COALESCE(last_message_id, 0), id) < ? RETURNING id",
        )
        .bind(cutoff)
        .fetch_all(&st.db)
        .await;
        match res {
            Ok(ids) => {
                for id in ids {
                    if let Ok(t) = db::channel(&st.db, id).await
                        && let Ok(audience) = db::audience(&st.db, &t).await
                    {
                        st.hub.send(&audience, &Event::ChannelUpdate(t.clone()));
                        let _ = starter_update(&st, &t).await;
                    }
                }
            }
            Err(e) => tracing::warn!("thread archiving: {e}"),
        }
    }
}

pub async fn create_dm(
    State(st): State<St>,
    auth: Auth,
    Json(req): Json<CreateDm>,
) -> R<Json<Channel>> {
    let me = auth.user;
    let mut others: Vec<Id> = req.recipients.into_iter().filter(|&u| u != me).collect();
    others.sort_unstable();
    others.dedup();
    if others.is_empty() || others.len() > 9 {
        return Err(bad("a DM has 1 to 9 other people"));
    }
    // Only people you already share a space or DM with: no cold messages to strangers.
    let known: HashSet<Id> = db::user_audience(&st.db, me).await?.into_iter().collect();
    if others.iter().any(|o| !known.contains(o)) {
        return Err(not_found("user"));
    }
    let key = (others.len() == 1).then(|| format!("{}:{}", me.min(others[0]), me.max(others[0])));
    if let Some(k) = &key {
        let existing: Option<Id> = sqlx::query_scalar("SELECT id FROM channels WHERE dm_key = ?")
            .bind(k)
            .fetch_optional(&st.db)
            .await?;
        if let Some(id) = existing {
            return Ok(Json(db::channel(&st.db, id).await?));
        }
    }
    let id = st.id();
    let kind = if key.is_some() {
        ChannelKind::Dm
    } else {
        ChannelKind::GroupDm
    };
    let mut tx = st.db.begin().await?;
    let inserted =
        sqlx::query("INSERT INTO channels (id, kind, name, dm_key) VALUES (?, ?, '', ?)")
            .bind(id)
            .bind(db::to_s(kind))
            .bind(&key)
            .execute(&mut *tx)
            .await;
    if let Err(sqlx::Error::Database(e)) = &inserted
        && e.is_unique_violation()
    {
        // Both people opened the DM at the same moment: use the other one.
        drop(tx);
        let id: Id = sqlx::query_scalar("SELECT id FROM channels WHERE dm_key = ?")
            .bind(&key)
            .fetch_one(&st.db)
            .await?;
        return Ok(Json(db::channel(&st.db, id).await?));
    }
    inserted?;
    for u in others.iter().chain([&me]) {
        sqlx::query("INSERT INTO dm_members (channel_id, user_id) VALUES (?, ?)")
            .bind(id)
            .bind(u)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let ch = db::channel(&st.db, id).await?;
    st.hub
        .send(&ch.recipients, &Event::ChannelCreate(ch.clone()));
    Ok(Json(ch))
}

#[cfg(test)]
mod tests {
    #[test]
    fn thread_names() {
        assert_eq!(
            super::thread_name_from("**hey** <@123> look at <#5> this a<b"),
            "hey look at this a<b"
        );
        assert_eq!(super::thread_name_from("<@1>"), "Thread");
        assert_eq!(super::thread_name_from(&"x".repeat(99)).len(), 40);
    }
}
