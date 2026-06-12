# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS builder

WORKDIR /app
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci

COPY dashboard ./
ARG VITE_API_BASE_URL=
ENV VITE_API_BASE_URL=${VITE_API_BASE_URL}
RUN npm run build

FROM nginx:1.27-alpine AS runtime

COPY deploy/docker/dashboard.nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/dist /usr/share/nginx/html

EXPOSE 80
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1/ >/dev/null || exit 1
