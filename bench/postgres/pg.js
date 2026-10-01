// Postgres workload, Bun's published "100 rows × 100 queries in flight":
// create a table, insert 100 rows, then fire 100 SELECTs at once and time
// how long the batch takes. The same node-postgres (`pg`) client runs on
// every runtime, so the comparison is the runtime's net/crypto (SCRAM-SHA-256
// auth) + event loop, not three different database drivers.
//
// Connection comes from PGHOST/PGPORT/PGUSER/PGPASSWORD/PGDATABASE (defaults
// target the bench's local container on 127.0.0.1:55432).
//
// Prints one line: "<batch_ms> <queries_per_s> ok|BAD".

const { Client } = require('pg');

const HOST = process.env.PGHOST || '127.0.0.1';
const PORT = Number(process.env.PGPORT || 55432);
const USER = process.env.PGUSER || 'bench';
const PASS = process.env.PGPASSWORD || 'bench';
const DB = process.env.PGDATABASE || 'bench';

async function main() {
  const client = new Client({
    host: HOST,
    port: PORT,
    user: USER,
    password: PASS,
    database: DB,
    ssl: false,
  });
  await client.connect();

  await client.query('DROP TABLE IF EXISTS bench_rows');
  await client.query(
    'CREATE TABLE bench_rows (id serial primary key, v int not null)'
  );
  for (let i = 0; i < 100; i++) {
    await client.query('INSERT INTO bench_rows (v) VALUES ($1)', [i]);
  }

  const t0 = process.hrtime.bigint();
  const results = await Promise.all(
    Array.from({ length: 100 }, (_, i) =>
      client.query('SELECT v FROM bench_rows WHERE id = $1', [i + 1])
    )
  );
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  const ok = results.length === 100 && results.every((r) => r.rows.length === 1);

  console.log(`${ms.toFixed(1)} ${Math.round(100 / (ms / 1000))} ${ok ? 'ok' : 'BAD'}`);
  await client.end();
  if (!ok) process.exit(1);
}

main().catch((e) => {
  console.error(`ERR ${e.code || ''} ${e.message}`);
  process.exit(1);
});