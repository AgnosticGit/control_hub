// Where `just demo` pretends its services run: two small virtual machines, catalog as three
// replicas spread over both. Container IDs are deterministic so restarts keep the same map.
import { createHash } from "node:crypto";

export const CATALOG_PORTS = [7006, 7016, 7026];

const PORTS = { gateway: 7001, orders: 7002, inventory: 7003, payments: 7004, notifications: 7005, auth: 7007 };

/** 64-hex container ID for a demo instance name. */
export function demoContainerId(name) {
  return createHash("sha256").update(`karbird-demo/${name}`).digest("hex");
}

const container = (name, port) => ({ name, id: demoContainerId(name), url: `http://127.0.0.1:${port}` });

/** Placement for the demo infra agent (`DEMO_INFRA_PLACEMENT`). */
export function demoPlacement() {
  return [
    {
      host: "vps-app",
      cores: 1,
      memoryMb: 1024,
      diskGb: 25,
      containers: [
        ...["gateway", "auth", "orders", "inventory"].map((n) => container(n, PORTS[n])),
        container("catalog-1", CATALOG_PORTS[0]),
        // orders and inventory reach the demo Postgres over 127.0.0.1, so the API places the
        // database in this container by its image (ADR-0018).
        {
          name: "postgres",
          id: demoContainerId("postgres"),
          image: "postgres:16-alpine",
          service: "postgres",
          synthetic: { cores: 0.03, rssMb: 96 },
        },
      ],
    },
    {
      host: "vps-worker",
      cores: 2,
      memoryMb: 2048,
      diskGb: 40,
      containers: [
        ...["payments", "notifications"].map((n) => container(n, PORTS[n])),
        container("catalog-2", CATALOG_PORTS[1]),
        container("catalog-3", CATALOG_PORTS[2]),
      ],
    },
  ];
}
