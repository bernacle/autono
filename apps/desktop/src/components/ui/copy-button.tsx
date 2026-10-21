import { useState } from "react";
import { Copy } from "lucide-react";
import { Button } from "./button";
export function CopyButton({
  text,
  label = "Copy",
}: {
  text: string;
  label?: string;
}) {
  const [state, setState] = useState("");
  return (
    <span className="copy-control">
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          void navigator.clipboard.writeText(text).then(
            () => setState("Copied"),
            () => setState("Copy unavailable"),
          );
        }}
      >
        <Copy size={13} />
        {state || label}
      </Button>
      <span className="sr-only" role="status">
        {state}
      </span>
    </span>
  );
}
