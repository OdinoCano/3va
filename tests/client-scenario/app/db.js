// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// The database module. Its top-level `await` is deliberate: the app must not
// start serving until the SCRAM handshake and the seed are done (the shape of a
// real `db/pg.js` that awaits a pool query before exporting).
import pg from 'pg';

const { Pool } = pg;

const pool = new Pool({
  host: process.env.PGHOST || 'db',
  port: Number(process.env.PGPORT || 5432),
  user: process.env.PGUSER || 'app',
  password: process.env.PGPASSWORD || 'appsecret',
  database: process.env.PGDATABASE || 'appdb',
  max: 5,
});

await pool.query('CREATE TABLE IF NOT EXISTS seed (id integer PRIMARY KEY, value text NOT NULL)');
await pool.query(
  "INSERT INTO seed (id, value) VALUES (1, 'seed-ok') ON CONFLICT (id) DO UPDATE SET value = EXCLUDED.value"
);

export const dbReady = true;

export async function getSeed() {
  const r = await pool.query('SELECT value FROM seed WHERE id = 1');
  return r.rows[0];
}
