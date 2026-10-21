import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, LockKeyhole } from "lucide-react";
import { useTransport } from "../lib/transport";
import type { Connection, LocalSettings } from "../lib/types";
import type { AppearanceSettings } from "../lib/appearance";
import { Empty, ErrorNotice, Panel } from "./common";
import { CopyButton } from "./ui/copy-button";
import { Segmented } from "./ui/segmented";
const sections = [
  "Daemon",
  "Agents",
  "Execution limits",
  "Verification",
  "Permissions",
  "Appearance",
] as const;
export function SettingsView({
  connection,
  appearance,
}: {
  connection: Connection | undefined;
  appearance: AppearanceSettings;
}) {
  const [section, setSection] = useState<(typeof sections)[number]>("Daemon");
  const transport = useTransport();
  const query = useQuery({
    queryKey: ["settings"],
    queryFn: () => transport.request<LocalSettings>({ type: "settings" }),
    retry: false,
  });
  const settings = query.data;
  return (
    <div className="settings-layout">
      <nav aria-label="Settings sections" className="settings-nav">
        {sections.map((s) => (
          <button
            key={s}
            aria-current={s === section ? "page" : undefined}
            onClick={() => setSection(s)}
          >
            {s}
          </button>
        ))}
      </nav>
      <div className="settings-content">
        <div className="readonly-banner">
          <LockKeyhole size={16} />
          <div>
            <strong>Read-only configuration</strong>
            <p>{settings?.config_path ?? "Local user configuration"}</p>
          </div>
          {settings && (
            <CopyButton label="Copy path" text={settings.config_path} />
          )}
        </div>
        {query.isError && (
          <ErrorNotice error={query.error} retry={() => void query.refetch()} />
        )}
        {section === "Daemon" && (
          <>
            <Panel title="Connection">
              <dl className="settings-dl">
                <dt>Connection</dt>
                <dd>
                  {connection ? (
                    <>
                      <Check className="success-text" size={13} />
                      Connected · {connection.address}
                    </>
                  ) : (
                    "Disconnected"
                  )}
                </dd>
                <dt>Daemon</dt>
                <dd>
                  {connection
                    ? `PID ${connection.host.pid} · API v${connection.host.api_version}`
                    : "Unavailable"}
                </dd>
                <dt>Autono home</dt>
                <dd>{connection?.home ?? settings?.home ?? "Unavailable"}</dd>
                <dt>Persistence</dt>
                <dd>
                  {connection
                    ? connection.host.persistence.healthy
                      ? "Healthy"
                      : "Degraded"
                    : "Unknown"}
                </dd>
                <dt>Stay awake</dt>
                <dd>
                  {connection
                    ? `${connection.host.power.enabled ? "Enabled" : "Disabled"} · assertion ${connection.host.power.held ? "held" : "not held"}`
                    : "Unknown"}
                </dd>
                {connection?.host.power.last_error && (
                  <>
                    <dt>Power error</dt>
                    <dd>{connection.host.power.last_error}</dd>
                  </>
                )}
              </dl>
            </Panel>
            {settings && (
              <Panel title="Local CLI availability">
                <div className="task-content">
                  {settings.cli.map((tool) => (
                    <div className="tool" key={tool.name}>
                      <code>{tool.name}</code>
                      <span>
                        {tool.available && (
                          <Check size={12} className="success-text" />
                        )}
                        {tool.available
                          ? "Found on desktop PATH"
                          : "Not on desktop PATH"}
                      </span>
                    </div>
                  ))}
                  <p className="meta">
                    Filesystem lookup only. Provider and authentication
                    readiness are reported by daemon preflight.
                  </p>
                </div>
              </Panel>
            )}
          </>
        )}
        {section === "Agents" && settings && (
          <Panel title="Agent roles">
            <table className="settings-table">
              <thead>
                <tr>
                  <th>Role</th>
                  <th>Provider</th>
                  <th>Model</th>
                  <th>Timeout</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(settings.agents).map(([role, agent]) => (
                  <tr key={role}>
                    <td>{role}</td>
                    <td>{agent.provider}</td>
                    <td>{agent.model ?? "Provider default"}</td>
                    <td>{agent.timeout_seconds}s</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Panel>
        )}
        {section === "Execution limits" && settings && (
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
              <dt>Pending submissions</dt>
              <dd>{settings.runner.max_pending_submissions}</dd>
              <dt>Stay awake preference</dt>
              <dd>{settings.runner.keep_awake ? "Enabled" : "Disabled"}</dd>
            </dl>
          </Panel>
        )}
        {section === "Verification" && settings && (
          <Panel title="Verification">
            <div className="task-content">
              <pre className="well">
                {JSON.stringify(settings.verification.commands, null, 2)}
              </pre>
              <p className="meta">
                {settings.verification.timeout_seconds}s per command · exact
                argument arrays, no implicit shell
              </p>
            </div>
          </Panel>
        )}
        {section === "Permissions" && (
          <Panel title="Configuration boundaries">
            <div className="task-content">
              <p>
                Permission rules are user-only configuration. Their exact
                operation and scope appear in each run's approval summary.
              </p>
              <p>
                Project autono.toml can override roles, execution, verification,
                preflight and stay-awake. Every run stores its effective
                immutable settings.
              </p>
              <p className="muted">
                Edit configuration in your editor and restart the daemon where
                required. This application does not write configuration files.
              </p>
              {settings && <p>{settings.source}</p>}
            </div>
          </Panel>
        )}
        {section === "Appearance" && (
          <Panel title="Appearance">
            <div className="task-content">
              <div className="row-between">
                <div>
                  <h3>Theme</h3>
                  <p className="meta">
                    Dark by default. System follows macOS live.
                  </p>
                </div>
                <Segmented
                  label="Appearance"
                  value={appearance.appearance}
                  onChange={appearance.setAppearance}
                  options={[
                    { value: "system", label: "System" },
                    { value: "dark", label: "Dark" },
                    { value: "light", label: "Light" },
                  ]}
                />
              </div>
              <label className="texture-choice">
                <div>
                  <strong>Background texture</strong>
                  <p className="meta">
                    Static grain on the window ground. Disabled with increased
                    contrast.
                  </p>
                </div>
                <input
                  type="checkbox"
                  role="switch"
                  checked={appearance.texture}
                  onChange={(e) => appearance.setTexture(e.target.checked)}
                />
              </label>
            </div>
          </Panel>
        )}
        {!settings && !query.isError && section !== "Appearance" && (
          <Empty title="Loading local configuration…" />
        )}
        <p className="meta">
          Appearance preferences apply only to this desktop. Project settings
          remain separate.
        </p>
      </div>
    </div>
  );
}
