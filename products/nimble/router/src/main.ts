import { loadConfig } from "./config";
import { startServer } from "./server";
import { Store } from "./store";

const config = loadConfig();
const store = new Store(config.db_path);
const server = startServer({ config, store });
console.log(
  `nimble listening on http://${server.hostname}:${server.port} · pool: ${config.routing.pool.join(", ")} · db: ${config.db_path}` +
    (process.env.NIMBLE_API_KEY ? " · bearer auth on" : " · no auth (loopback)"),
);
