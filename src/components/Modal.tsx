import { useEffect } from "react";
import { createPortal } from "react-dom";
import "./Modal.css";

export const Modal = ({
  isOpen,
  close,
  sizeFit,
  children,
  hideClose,
  zIndex,
}: {
  children: React.ReactNode;
  isOpen: boolean;
  close?: () => void;
  sizeFit?: boolean;
  hideClose?: boolean;
  zIndex?: number;
}) => {
  useEffect(() => {
    if (!isOpen || !close) {
      return;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [isOpen, close]);

  if (!isOpen) return null;

  // Dialogs must stay usable when their retained setup panel is hidden by the
  // access gate while an Apple verification or device pairing is in progress.
  return createPortal(
    <div className="modal-container" style={zIndex ? { zIndex } : {}}>
      <div className={`modal${sizeFit ? " size-fit" : ""}`} role="dialog" aria-modal="true">
        {!hideClose && close && (
          <button className="modal-close" onClick={close} aria-label="Close dialog">
            &#x2715;
          </button>
        )}
        <div className="modal-content">{children}</div>
      </div>
    </div>,
    document.body,
  );
};
