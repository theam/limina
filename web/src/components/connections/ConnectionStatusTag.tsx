import { CheckmarkFilled, WarningAltFilled } from "@carbon/icons-react";
import { connectionStatus, type ConnectionStatus } from "../../lib/connections";
import { Tag } from "../ui/Status";

export function ConnectionStatusTag({ status }: { status: ConnectionStatus }) {
  const meta = connectionStatus(status);
  return (
    <Tag tone={meta.tone}>
      {status === "CONNECTED" ? (
        <CheckmarkFilled size={12} />
      ) : status === "NOT_CONNECTED" ? null : (
        <WarningAltFilled size={12} />
      )}
      {meta.label}
    </Tag>
  );
}
