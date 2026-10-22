# The listener. Built from the repo root, so that the workspace's lockfile and catalog
# apply: docker build -f apps/server/Dockerfile .
FROM oven/bun:1.4.2

WORKDIR /app

# Every workspace's package.json, so that a frozen install matches bun.lock.
COPY package.json bun.lock turbo.json ./
COPY apps/server/package.json apps/server/
COPY apps/desktop/package.json apps/desktop/
COPY apps/home-assistant/package.json apps/home-assistant/
COPY packages/scent-core/package.json packages/scent-core/
COPY tools/package.json tools/
RUN bun install --frozen-lockfile --production --filter '@scent-control/server'

COPY apps/server apps/server

USER bun
WORKDIR /app/apps/server
CMD ["bun", "run", "src/main.ts"]
