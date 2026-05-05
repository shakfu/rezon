import { Select } from "@base-ui/react/select";

type Item = { value: string; label: string };

type Props = {
  items: Item[];
  value: string;
  onValueChange: (v: string) => void;
  placeholder?: string;
  className?: string;
};

const TRIGGER =
  "flex w-full items-center justify-between rounded-md border border-border bg-transparent px-2 py-1.5 text-[13px] text-fg outline-none cursor-pointer hover:bg-bg-soft focus-visible:ring-2 focus-visible:ring-accent data-[popup-open]:ring-2 data-[popup-open]:ring-accent";

const POPUP =
  "z-[150] max-h-64 min-w-[var(--anchor-width)] overflow-y-auto rounded-md border border-border bg-bg-elev py-1 text-[13px] text-fg shadow-[0_10px_30px_rgba(0,0,0,0.3)]";

const ITEM =
  "flex cursor-pointer items-center gap-2 px-2.5 py-1.5 outline-none data-[highlighted]:bg-accent-soft";

export function BaseSelect({
  items,
  value,
  onValueChange,
  placeholder,
  className = "",
}: Props) {
  return (
    <Select.Root
      items={items}
      value={value}
      onValueChange={(v) => v != null && onValueChange(v)}
    >
      <Select.Trigger className={`${TRIGGER} ${className}`}>
        <Select.Value placeholder={placeholder} />
        <Select.Icon className="ml-2 text-fg-dim">▾</Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner sideOffset={4}>
          <Select.Popup className={POPUP}>
            <Select.List>
              {items.map(({ value: v, label }) => (
                <Select.Item key={v} value={v} className={ITEM}>
                  <Select.ItemIndicator className="w-3 text-accent">
                    ✓
                  </Select.ItemIndicator>
                  <Select.ItemText>{label}</Select.ItemText>
                </Select.Item>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}
