-- Roles: named, coloured permission sets per space (Discord-style). `@everyone` lives on the space itself.
-- 63552 = perm::EVERYONE_DEFAULT (create invites, send messages, attach files, add reactions, create threads, connect).
ALTER TABLE spaces ADD COLUMN everyone_permissions INTEGER NOT NULL DEFAULT 63552;

CREATE TABLE roles (
    id          INTEGER PRIMARY KEY,
    space_id    INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    color       TEXT    NOT NULL DEFAULT '',
    emoji       TEXT    NOT NULL DEFAULT '',
    position    INTEGER NOT NULL,
    permissions INTEGER NOT NULL DEFAULT 0,
    hoist       INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX roles_space ON roles(space_id, position);

CREATE TABLE member_roles (
    space_id INTEGER NOT NULL,
    user_id  INTEGER NOT NULL,
    role_id  INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (space_id, user_id, role_id),
    FOREIGN KEY (space_id, user_id) REFERENCES members(space_id, user_id) ON DELETE CASCADE
);
CREATE INDEX member_roles_role ON member_roles(role_id);

-- Admins of the old owner/admin/member scheme get a real "Admin" role at start-up (db::upgrade_roles), which needs
-- the server's id generator; members.role keeps only 'owner' or 'member' after that.
