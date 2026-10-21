import { Component, type ReactNode } from "react";
import { Button } from "./ui/button";
/** A rendering fault must not strand the window or imply daemon work stopped. */
export class ViewBoundary extends Component<
  { children: ReactNode; reload?: () => void },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    if (this.state.failed)
      return (
        <main className="desktop-startup" role="alert">
          <h1>Desktop view could not render</h1>
          <p>
            The daemon owns running work independently. Reload this view to
            reconnect.
          </p>
          <Button
            onClick={() =>
              this.props.reload ? this.props.reload() : window.location.reload()
            }
          >
            Reload view
          </Button>
        </main>
      );
    return this.props.children;
  }
}
