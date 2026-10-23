export interface GatewayMetricSnapshot {
  databaseReady: boolean;
  upstreamConfigured: boolean;
  generationEnabled: boolean;
  inflightRequests: number;
  configurationSource: "environment" | "database";
}

const gauge = (name: string, help: string, value: number): string =>
  `# HELP ${name} ${help}\n# TYPE ${name} gauge\n${name} ${value}\n`;

export function renderMetrics(snapshot: GatewayMetricSnapshot): string {
  return [
    gauge("inver_gateway_database_ready", "Whether the SQLite database is responsive.", snapshot.databaseReady ? 1 : 0),
    gauge("inver_gateway_upstream_configured", "Whether at least one public model is routable.", snapshot.upstreamConfigured ? 1 : 0),
    gauge("inver_gateway_generation_enabled", "Whether new generation requests are enabled.", snapshot.generationEnabled ? 1 : 0),
    gauge("inver_gateway_inflight_requests", "Current metered generation requests.", snapshot.inflightRequests),
    `# HELP inver_gateway_upstream_configuration Source of the active upstream configuration.\n`,
    `# TYPE inver_gateway_upstream_configuration gauge\n`,
    `inver_gateway_upstream_configuration{source="${snapshot.configurationSource}"} 1\n`,
  ].join("");
}
