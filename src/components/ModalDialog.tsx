import { X } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";

import "./modal-dialog.css";

interface ModalDialogProps {
  title: string;
  titleId: string;
  onClose: () => void;
  children: ReactNode;
  className?: string;
}

export function ModalDialog({
  title,
  titleId,
  onClose,
  children,
  className = "",
}: ModalDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    const trigger = document.activeElement;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) {
        trigger.focus();
      }
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      className={`modal-dialog ${className}`}
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <header className="modal-dialog-heading">
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label={`Close ${title}`}
          onClick={onClose}
          autoFocus
        >
          <X size={18} />
        </button>
      </header>
      {children}
    </dialog>
  );
}
