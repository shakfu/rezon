import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";
import type { ReactNode } from "react";

const POPUP =
  "tooltip z-[200] select-none rounded bg-fg px-2 py-1 text-[11px] leading-none text-bg";

type Side = "left" | "right" | "top" | "bottom";

type Props = {
  label: ReactNode;
  side?: Side;
  sideOffset?: number;
  className?: string;
  onClick?: () => void;
  children: ReactNode;
};

export function Tooltip({
  label,
  side = "right",
  sideOffset = 6,
  className,
  onClick,
  children,
}: Props) {
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger className={className} onClick={onClick}>
        {children}
      </TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Positioner side={side} sideOffset={sideOffset}>
          <TooltipPrimitive.Popup className={POPUP}>
            {label}
            <TooltipPrimitive.Arrow className="fill-fg" />
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}
