import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp, X } from "lucide-react";
import { ProcessLogs } from "./activity";
import { Button } from "./ui/button";
export function LogDock({ id, onClose }: { id: string; onClose: () => void }) {
  const [large, setLarge] = useState(false),
    close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement;
    close.current?.focus({ preventScroll: true });
    return () => {
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, [id]);
  return (
    <section
      id="process-log-dock"
      className={`log-dock ${large ? "large" : ""}`}
      aria-label="Process log dock"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <header>
        <strong>Process output</strong>
        <span className="meta">Sanitized daemon evidence</span>
        <div />
        <Button
          variant="ghost"
          size="icon"
          aria-label={large ? "Reduce log dock" : "Expand log dock"}
          onClick={() => setLarge((v) => !v)}
        >
          {large ? <ChevronDown size={15} /> : <ChevronUp size={15} />}
        </Button>
        <Button
          ref={close}
          variant="ghost"
          size="icon"
          aria-label="Close log dock"
          onClick={onClose}
        >
          <X size={15} />
        </Button>
      </header>
      <div className="log-dock-body">
        <ProcessLogs key={id} id={id} />
      </div>
    </section>
  );
}
