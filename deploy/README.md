# Deploying eddy-server

The server is optional: without it, every client connects to FreshRSS directly and works on its own. Once deployed, it handles cross-device sync, search over the full history, and distribution of private rule packs and AI settings. It also takes over the FreshRSS account.

## Running locally (OrbStack or Docker Desktop)

```sh
cd deploy
cp .env.example .env
# Generate the master key and write it into .env (once; back it up with the data)
sed -i '' "s|^EDDY_MASTER_KEY=.*|EDDY_MASTER_KEY=$(openssl rand -base64 32)|" .env
docker compose up -d --build
curl http://127.0.0.1:8787/v1/health
```

The port is mapped to `127.0.0.1:8787` only.

## Pairing devices

Each device needs a one-time pairing code (valid for 15 minutes by default):

```sh
docker compose exec eddy-server eddy-server pairing-code
```

Enter the server address and the pairing code in Eddy's settings. If the first device to connect is already using FreshRSS in standalone mode, the account is handed over to the server, and its local read state, stars, reading progress, saved pages and tags are uploaded and merged.

Managing devices:

```sh
docker compose exec eddy-server eddy-server devices
docker compose exec eddy-server eddy-server remove-device <device ID>
```

## Data and backups

- Data lives in the named volume `eddy-data` (`/data` inside the container): the SQLite database, content blobs, the search index and the encrypted FreshRSS password (`secrets.json`).
- If `EDDY_MASTER_KEY` is lost, the FreshRSS password can no longer be decrypted; re-enter the account on a client. Other data is unaffected.
- To run video-to-book jobs in the background on the server (so a client can close its lid and go offline while waiting), set `EDDY_VIDEO_STAGE2_*` in `.env`, and optionally `EDDY_VIDEO_STAGE1_*` and `EDDY_VIDEO_ASR_*`. The keys stay in the server's environment; clients only read the boolean `video_capable`.

## Moving to a server

The same image runs on the machine that hosts FreshRSS (`linux/amd64` and `linux/arm64`). How to reach it remotely (public HTTPS behind a reverse proxy, or a private network) will be decided at the iOS stage; until then, keep it listening on the loopback address only.
