# eddy-server 部署

服务端是可选的：不部署时每个客户端都能直连 FreshRSS 单机使用。部署后它负责跨设备同步、全量历史搜索，以及私有规则包和 AI 设置的下发；FreshRSS 账号也由它接管。

## 本机运行（OrbStack 或 Docker Desktop）

```sh
cd deploy
cp .env.example .env
# 生成主密钥写进 .env（只需一次，和数据一起备份）
sed -i '' "s|^EDDY_MASTER_KEY=.*|EDDY_MASTER_KEY=$(openssl rand -base64 32)|" .env
docker compose up -d --build
curl http://127.0.0.1:8787/v1/health
```

端口只映射到 `127.0.0.1:8787`。

## 配对设备

每台设备需要一个一次性配对码（默认 15 分钟有效）：

```sh
docker compose exec eddy-server eddy-server pairing-code
```

在 Eddy 的设置里填服务端地址和配对码。第一台连接的设备如果已经在单机模式下连着 FreshRSS，账号会交给服务端，本地的已读、收藏、阅读进度、保存的网页、标签都会上传合并。

管理设备：

```sh
docker compose exec eddy-server eddy-server devices
docker compose exec eddy-server eddy-server remove-device <设备 ID>
```

## 数据与备份

- 数据在命名卷 `eddy-data`（容器内 `/data`）：SQLite 数据库、内容 blob、搜索索引、加密后的 FreshRSS 密码（`secrets.json`）。
- `EDDY_MASTER_KEY` 丢失后无法解密 FreshRSS 密码，只需在客户端重新填写账号；其他数据不受影响。

## 迁到服务器

同一个镜像可以在 FreshRSS 所在的服务器上运行（`linux/amd64` 和 `linux/arm64`）。远程访问方式（公网 HTTPS 加反向代理，或私有网络）到 iOS 阶段再定；在那之前保持只监听回环地址。
