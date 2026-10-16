// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors
//
// The app the scenario runs with the released 3va binary: Express 4 with
// express.json(), a session login, a multer upload whose SHA-256 can be
// downloaded back, an outgoing axios call to a mock upstream, and an
// /ip route behind nginx that reads the address 3va resolved from
// X-Forwarded-For through `trustedProxies`.
import express from 'express';
import session from 'express-session';
import multer from 'multer';
import axios from 'axios';
import crypto from 'crypto';
import { dbReady, getSeed } from './db.js';

const app = express();
app.set('trust proxy', 1);
app.use(express.json());
app.use(
  session({
    secret: process.env.SESSION_SECRET || 'client-scenario',
    resave: false,
    saveUninitialized: true,
    cookie: { httpOnly: true, sameSite: 'lax' },
  })
);

const upload = multer({ storage: multer.memoryStorage() });
const files = new Map();

app.post('/login', (req, res) => {
  const { user, password } = req.body || {};
  if (user === 'demo' && password === 'secret') {
    req.session.user = user;
    return res.json({ ok: true, user });
  }
  return res.status(401).json({ ok: false });
});

app.get('/me', (req, res) => {
  if (req.session.user) return res.json({ ok: true, user: req.session.user });
  return res.status(401).json({ ok: false });
});

app.post('/upload', upload.single('file'), (req, res) => {
  const f = req.file;
  const sha = crypto.createHash('sha256').update(f.buffer).digest('hex');
  const id = String(files.size + 1);
  files.set(id, { buf: f.buffer, sha, name: f.originalname });
  res.json({ id, sha, size: f.size });
});

app.get('/download/:id', (req, res) => {
  const f = files.get(req.params.id);
  if (!f) return res.status(404).end();
  res.setHeader('content-type', 'application/octet-stream');
  res.setHeader('content-length', String(f.buf.length));
  return res.end(f.buf);
});

app.get('/upstream', async (req, res) => {
  try {
    const r = await axios.get((process.env.MOCK_URL || 'http://mock:1234') + '/data', { timeout: 5000 });
    res.json({ ok: true, upstream: r.data });
  } catch (e) {
    res.status(502).json({ ok: false, error: e.message });
  }
});

app.get('/ip', (req, res) => {
  res.json({ remoteAddress: req.socket.remoteAddress, ip: req.ip });
});

app.get('/db', async (req, res) => {
  const row = await getSeed();
  res.json({ ok: true, row, dbReady });
});

app.get('/healthz', (req, res) => res.json({ ok: true }));

const port = Number(process.env.PORT || 8080);
app.listen(port, '0.0.0.0', () => console.log('READY'));
