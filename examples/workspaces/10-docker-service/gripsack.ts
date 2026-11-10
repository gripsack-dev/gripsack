import {
  defineWorkspace, exec, lit, service, task, workspace,
} from "@gripsack/core";

const postgres = service("postgres", {
  image: "docker.io/library/postgres@sha256:979bccfde35445db618c65107d27d08c6c21339d8dc3ad4c830c6633947be0e4",
  ports: [{ address: "127.0.0.1", host: 55432, container: 5432 }],
  volumes: [{ name: "gripsack-guide-postgres18", target: "/var/lib/postgresql" }],
  env: {
    POSTGRES_USER: { kind: "literal", value: "app" },
    POSTGRES_DB: { kind: "literal", value: "app" },
    POSTGRES_PASSWORD: { kind: "inherited", name: "GRIPSACK_POSTGRES_PASSWORD" },
  },
  health: {
    kind: "command",
    argv: ["pg_isready", "-U", "app", "-d", "app"],
    interval_seconds: 2,
    timeout_seconds: 2,
    retries: 30,
  },
});
const databaseReady = task("database-ready", {
  services: ["postgres"],
  steps: [exec({ argv: [lit("/usr/bin/printf"), lit("postgres is healthy\\n")] })],
});

export default defineWorkspace(() => workspace({ outputs: [postgres, databaseReady] }));
