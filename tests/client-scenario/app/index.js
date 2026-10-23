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
import http from 'http';
import net from 'net';
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

// The outgoing request has to go through the proxy of the network: the agent
// builds the connection (the `CONNECT` tunnel) and the request is written on
// it. If the runtime ignored the agent, the request would leave the machine
// without the proxy.
app.get('/upstream-via-proxy', async (req, res) => {
  const proxyUrl = new URL(process.env.PROXY_URL || 'http://proxy:1235');
  const agent = new http.Agent();
  // The factory that builds the tunnel: the request is written on it, so it
  // leaves the machine through the proxy.
  agent.createConnection = function createConnection(options, cb) {
    const socket = net.connect(Number(proxyUrl.port), proxyUrl.hostname, () => {
      socket.write(
        'CONNECT ' + options.host + ':' + options.port + ' HTTP/1.1\r\n' +
        'Host: ' + options.host + ':' + options.port + '\r\n\r\n',
      );
    });
    let head = '';
    const onData = (chunk) => {
      head += chunk.toString('latin1');
      const idx = head.indexOf('\r\n\r\n');
      if (idx < 0) return;
      socket.removeListener('data', onData);
      const status = (head.split('\r\n')[0] || '').split(' ')[1];
      if (status !== '200') {
        socket.destroy();
        cb(new Error('the proxy refused the tunnel: ' + status));
        return;
      }
      cb(null, socket);
    };
    socket.on('data', onData);
    socket.on('error', (err) => cb(err));
  };
  try {
    const body = await new Promise((resolve, reject) => {
      // `/data` is the mock's only JSON route: the bare URL asked for `/` and got
      // an empty 404, which no runtime (Node included) can parse.
      const r = http.get((process.env.MOCK_URL || 'http://mock:1234') + '/data', { agent }, (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
      });
      r.on('error', reject);
    });
    agent.destroy();
    res.json({ ok: true, upstream: JSON.parse(body) });
  } catch (e) {
    agent.destroy();
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
