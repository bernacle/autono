import * as Dropdown from "@radix-ui/react-dropdown-menu";
import { MoreHorizontal } from "lucide-react";
export function Menu({
  label = "More actions",
  items,
}: {
  label?: string;
  items: {
    label: string;
    onSelect: () => void;
    disabled?: boolean;
    destructive?: boolean;
  }[];
}) {
  return (
    <Dropdown.Root>
      <Dropdown.Trigger
        className="button button-ghost button-icon"
        aria-label={label}
      >
        <MoreHorizontal size={17} />
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="popover menu" align="end" sideOffset={6}>
          {items.map((item) => (
            <Dropdown.Item
              key={item.label}
              onSelect={item.onSelect}
              disabled={item.disabled}
              className={item.destructive ? "danger-text" : ""}
            >
              {item.label}
            </Dropdown.Item>
          ))}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}
