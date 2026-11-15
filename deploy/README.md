# Deploying eddy-server

The server is optional: without it, every client connects to FreshRSS directly and works on its own. Once deployed, it handles cross-device sync, search over the full history, and distribution of private rule packs and AI settings. It reads FreshRSS on behalf of every device; a device that also has the account keeps it only to fetch new articles while the server is offline.

## Running locally (OrbStack or Docker Desktop)

```sh
cd deploy
cp .env.example .env
# Generate the master key and write it into .env (once; back it up with the data)
sed -i '' "s|^EDDY_MASTER_KEY=.*|EDDY_MASTER_KEY=$(openssl rand -base64 32)|" .env
EDDY_GIT_HASH=$(git rev-parse --short HEAD) docker compose up -d --build
curl http://127.0.0.1:8787/v1/health
docker compose exec eddy-server eddy-server --version
```

The port is mapped to `127.0.0.1:8787` only, so nothing outside this machine can connect (see [Other devices on the local network](#other-devices-on-the-local-network)). `docker compose ps` shows the container as `healthy` once the database answers.

To check a build end to end before trusting it with data, run `scripts/test-server-smoke.sh` from the repository root: it starts a throwaway server, pairs two `eddy-cli` devices with it, stops it, makes changes offline, starts it again and checks that both devices converge.

## Prebuilt images

Instead of building, the image can be pulled from GitHub's registry:

| Tag | What it is |
|---|---|
| `ghcr.io/wheheohu/eddy/eddy-server:X.Y.Z`, `:latest` | A core release (`linux/amd64` and `linux/arm64`) |
| `:main`, `:dev` | The newest commit of that branch that passed CI (`linux/amd64`) |
| `:sha-<commit>` | One specific commit of `dev` or `main` (`linux/amd64`) |

To use one, replace the `build:` section of `compose.yaml` with `image: ghcr.io/wheheohu/eddy/eddy-server:<tag>`. While the package is private, sign in first with `docker login ghcr.io` and a token that can read packages.

## Admin page

Set `EDDY_ADMIN_PASSWORD` in `.env` and restart (`docker compose up -d`) to turn on the admin page at <http://127.0.0.1:8787/admin>. Signed in with that password you can:

- create pairing codes, see paired devices and when each last synced, and revoke one;
- see whether FreshRSS is in step (last refresh, last error, feeds and articles), refresh now, and set or change the account;
- watch video-to-book jobs, queue a link, and retry, cancel or clear jobs;
- check the service: version, uptime, connected devices and disk use.

Without the variable the page does not exist (`/admin` answers 404) and everything below still works from the command line. A pairing code gives a new device everything on the server, so treat the password accordingly: sign-in attempts slow down after a few wrong ones, a session lasts 12 hours and ends when the server restarts, and if the server is ever reachable from outside this machine it must be behind HTTPS.

## Pairing devices

Each device needs a one-time pairing code (valid for 15 minutes by default), from the admin page or:

```sh
docker compose exec eddy-server eddy-server pairing-code
```

Enter the server address and the pairing code in Eddy's settings. If the first device to connect is already using FreshRSS in standalone mode, the server is given the account, and the device's read state, stars, reading progress, saved pages, books and tags are uploaded and merged. Back up the device's data directory first (`scripts/backup-app-data.sh`).

The server must reach FreshRSS at exactly the address the device uses. Article IDs are derived from that address and the username, so a different spelling of the same server (`localhost` on the Mac, a container name on the server) would show every feed twice; pairing refuses when the server already has a different account.

A server that has no account yet can also be given one later, from Settings → Accounts in server mode or with `eddy-cli server-account URL USER PASSWORD`.

Managing devices:

```sh
docker compose exec eddy-server eddy-server devices
docker compose exec eddy-server eddy-server remove-device <device ID>
```

## When the server is offline

Devices keep working: reading, read/star, saving pages, importing, tags, local search and AI need no server. Changes queue on the device and upload by themselves when the server is back; Settings → Accounts shows how many are waiting. A device that has the FreshRSS account still receives new articles, straight from FreshRSS. Search covers only what is on the device, and content another device created that was never downloaded cannot be opened until the server returns.

## Starting over with an empty server

A test server can be thrown away and recreated without touching the devices' data:

```sh
docker compose down -v      # deletes the server's data volume
docker compose up -d
docker compose exec eddy-server eddy-server pairing-code
```

Disconnect in Eddy's settings and pair again with the new code. The device notices it is a server it has not synced with and uploads its whole history again.

## Versions

`eddy-server --version` prints the core version and commit; clients show it in Settings. Client and server do not need the same version: they sync as long as their sync protocols are compatible, and each tells the other to update when they are not (see [`docs/adr/0009`](../docs/adr/0009-versions-and-compatibility.md)). Update the server before the devices when a release raises the protocol version.

## Data and backups

- Data lives in the named volume `eddy-data` (`/data` inside the container): the SQLite database, content blobs, the search index and the encrypted FreshRSS password (`secrets.json`).
- If `EDDY_MASTER_KEY` is lost, the FreshRSS password can no longer be decrypted; re-enter the account on a client. Other data is unaffected.
- To run video-to-book jobs in the background on the server (so a client can close its lid and go offline while waiting), set `EDDY_VIDEO_STAGE2_*` in `.env`, and optionally `EDDY_VIDEO_STAGE1_*` and `EDDY_VIDEO_ASR_*`. The keys stay in the server's environment; clients only read the boolean `video_capable`.

## Other devices on the local network

A device on another machine gets "cannot reach the server" while the port is published on the loopback address, and the server logs nothing, because the connection never arrives. To let devices on a network you trust connect, set the address to publish on in `.env` and recreate the container:

```sh
# This machine's address on that network, or 0.0.0.0 for every interface
EDDY_PUBLISH_ADDRESS=192.168.1.20
```

```sh
docker compose up -d
docker compose ps    # PORTS shows the new address
```

Devices then pair with `http://<that address>:8787`. The server speaks plain HTTP: pairing codes, device tokens and everything synced cross the network unencrypted, and the admin page is reachable the same way. Keep this to a network you control, and set it back to `127.0.0.1` when you are done.

## Moving to a server

The same image runs on the machine that hosts FreshRSS (`linux/amd64` and `linux/arm64`). How to reach it remotely (public HTTPS behind a reverse proxy, or a private network) will be decided at the iOS stage; until then, keep it on the loopback address or a local network you trust.
