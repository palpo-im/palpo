FROM node:24-alpine@sha256:e67514e5d0f6c46656005e1b693b2ec9d52e80b641307de684d4a015ba7a4eaf
WORKDIR /app
RUN mkdir /app/data && chown node:node /app/data && chmod 700 /app/data
COPY --chown=node:node package.json server.mjs ./
COPY --chown=node:node lib ./lib
COPY --chown=node:node public ./public
USER node
ENV NODE_ENV=production PORT=8090 LISTEN_HOST=0.0.0.0 PALPO_ADMIN_DATABASE=/app/data/admin.sqlite
EXPOSE 8090
CMD ["node", "server.mjs"]
