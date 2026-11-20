-- Nextcloud sign-in and file storage.

ALTER TABLE users ADD COLUMN nc_user_id TEXT;
CREATE UNIQUE INDEX users_nc ON users(nc_user_id) WHERE nc_user_id IS NOT NULL;

-- Set for sessions that signed in through Nextcloud; the app password never leaves the server.
ALTER TABLE sessions ADD COLUMN nc_login TEXT;
ALTER TABLE sessions ADD COLUMN nc_app_password TEXT;

-- Files kept in the uploader's Nextcloud: absolute download URL, share page, and the share to remove on delete.
ALTER TABLE uploads ADD COLUMN url TEXT;
ALTER TABLE uploads ADD COLUMN link TEXT;
ALTER TABLE uploads ADD COLUMN nc_share_id TEXT;
