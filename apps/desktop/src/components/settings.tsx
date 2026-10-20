import { useQuery } from "@tanstack/react-query";
import { useTransport } from "../lib/transport";
import type { Connection, LocalSettings } from "../lib/types";
import { Empty, ErrorNotice, Panel } from "./common";
export function SettingsView({
  connection,
}: {
  connection: Connection | undefined;
}) {
  const transport = useTransport();
  const query = useQuery({
    queryKey: ["settings"],
    queryFn: () => transport.request<LocalSettings>({ type: "settings" }),
    retry: false,
  });
  const settings = query.data;
  return (
    <>
      <div className="page-heading">
        <div>
          <div className="eyebrow">Local environment</div>
          <h1>Settings</h1>
          <p>
            Read-only configuration inspection. Run snapshots are shown before
            approval.
          </p>
        </div>
      </div>
      {query.isError && (
        <ErrorNotice error={query.error} retry={() => void query.refetch()} />
      )}
      <Panel title="Connected daemon">
        <dl className="settings-dl">
          <dt>Address</dt>
          <dd>{connection?.address ?? "Disconnected"}</dd>
          <dt>Autono home</dt>
          <dd>{connection?.home ?? settings?.home ?? "Unavailable"}</dd>
          <dt>Daemon process</dt>
          <dd>
            {connection
              ? `${connection.host.pid} · API v${connection.host.api_version}`
              : "Unavailable"}
          </dd>
          <dt>Persistence</dt>
          <dd>
            {connection?.host.persistence.healthy
              ? "Healthy"
              : connection
                ? "Degraded"
                : "Unknown"}
          </dd>
          <dt>Stay awake</dt>
          <dd>
            {connection
              ? `${connection.host.power.enabled ? "Enabled" : "Disabled"} · assertion ${connection.host.power.held ? "held" : "not held"}`
              : "Unknown"}
          </dd>
        </dl>
      </Panel>
      {settings ? (
        <>
          <Panel title="Configuration boundaries">
            <div className="task-content">
              <code>{settings.config_path}</code>
              <p>{settings.source}</p>
              <p>
                User-only: permissions, daemon and process admission. Project{" "}
                <code>autono.toml</code> can override roles, execution,
                verification, preflight and stay-awake. Each run stores its
                effective immutable settings.
              </p>
              <p className="muted">
                Edit configuration using your editor and restart the daemon
                where required. This version does not write configuration files.
              </p>
            </div>
          </Panel>
          <Panel title="Configured agent roles">
            <div className="settings-roles">
              {Object.entries(settings.agents).map(([role, agent]) => (
                <div key={role}>
                  <span className="eyebrow">{role}</span>
                  <h3>{agent.provider}</h3>
                  <p>{agent.model ?? "Provider default model"}</p>
                  <small>{agent.timeout_seconds}s timeout</small>
                </div>
              ))}
            </div>
          </Panel>
          <div className="two-columns">
            <Panel title="Execution limits">
              <dl className="settings-dl">
                <dt>Parallel tasks</dt>
                <dd>{settings.execution.max_parallel}</dd>
                <dt>Coding attempts</dt>
                <dd>{settings.execution.max_attempts}</dd>
                <dt>Review cycles</dt>
                <dd>{settings.execution.max_review_cycles}</dd>
                <dt>Process capacity</dt>
                <dd>{settings.runner.max_active_runs}</dd>
                <dt>Stay awake preference</dt>
                <dd>{settings.runner.keep_awake ? "Enabled" : "Disabled"}</dd>
              </dl>
            </Panel>
            <Panel title="Local CLI availability">
              <div className="task-content">
                {settings.cli.map((tool) => (
                  <div className="tool" key={tool.name}>
                    <code>{tool.name}</code>
                    <span
                      className={
                        tool.available ? "success-text" : "warning-text"
                      }
                    >
                      {tool.available
                        ? "Found on desktop PATH"
                        : "Not on desktop PATH"}
                    </span>
                  </div>
                ))}
                <p className="footnote">
                  Filesystem lookup only. No commands run on launch. Provider
                  capability and authentication readiness are reported by daemon
                  preflight.
                </p>
              </div>
            </Panel>
          </div>
          <Panel title="Default verification">
            <div className="task-content">
              <pre>
                {JSON.stringify(settings.verification.commands, null, 2)}
              </pre>
              <small>
                {settings.verification.timeout_seconds}s per command · argument
                arrays, no implicit shell
              </small>
            </div>
          </Panel>
        </>
      ) : (
        !query.isError && <Empty title="Loading local configuration…" />
      )}
    </>
  );
}
