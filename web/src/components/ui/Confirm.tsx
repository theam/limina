import * as Dialog from "@radix-ui/react-dialog";

/** A yes/no dialog for actions that are hard to undo. */
export function Confirm({
  open,
  onOpenChange,
  title,
  body,
  action,
  onConfirm,
  pending,
  danger,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  body: string;
  action: string;
  onConfirm: () => void;
  pending?: boolean;
  /** Style the action as destructive. */
  danger?: boolean;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content className="dialog">
          <div className="dialog-header">
            <Dialog.Title className="dialog-title">{title}</Dialog.Title>
            <Dialog.Description className="dialog-description">{body}</Dialog.Description>
          </div>
          <div className="dialog-footer" style={{ marginTop: "var(--space-5)" }}>
            <Dialog.Close className="btn">Cancel</Dialog.Close>
            <button className={danger ? "btn btn-danger" : "btn btn-primary"} onClick={onConfirm} disabled={pending}>
              {pending && <span className="spinner" />}
              {action}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
