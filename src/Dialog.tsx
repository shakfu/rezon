import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { AlertDialog as AlertDialogPrimitive } from "@base-ui/react/alert-dialog";
import type { ComponentProps } from "react";

const BACKDROP =
  "dialog-overlay fixed inset-0 z-[100] bg-black/40";

const POPUP =
  "dialog fixed top-1/2 left-1/2 z-[101] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-bg-elev text-fg shadow-[0_20px_60px_rgba(0,0,0,0.4)]";

export const Dialog = DialogPrimitive;
export const AlertDialog = AlertDialogPrimitive;

export function DialogBackdrop(
  props: ComponentProps<typeof DialogPrimitive.Backdrop>,
) {
  const { className = "", ...rest } = props;
  return (
    <DialogPrimitive.Backdrop
      {...rest}
      className={`${BACKDROP} ${className}`.trim()}
    />
  );
}

export function DialogPopup(
  props: ComponentProps<typeof DialogPrimitive.Popup>,
) {
  const { className = "", ...rest } = props;
  return (
    <DialogPrimitive.Popup
      {...rest}
      className={`${POPUP} ${className}`.trim()}
    />
  );
}

export function AlertDialogBackdrop(
  props: ComponentProps<typeof AlertDialogPrimitive.Backdrop>,
) {
  const { className = "", ...rest } = props;
  return (
    <AlertDialogPrimitive.Backdrop
      {...rest}
      className={`${BACKDROP} ${className}`.trim()}
    />
  );
}

export function AlertDialogPopup(
  props: ComponentProps<typeof AlertDialogPrimitive.Popup>,
) {
  const { className = "", ...rest } = props;
  return (
    <AlertDialogPrimitive.Popup
      {...rest}
      className={`${POPUP} ${className}`.trim()}
    />
  );
}
