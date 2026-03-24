import type { MissionState } from "./mission";

interface SlackMessage {
  text: string;
  blocks?: Array<{
    type: string;
    text?: { type: string; text: string };
    elements?: Array<{ type: string; text: string }>;
  }>;
}

/**
 * Send a Slack webhook notification.
 * Non-blocking — failures are logged but never crash the mission.
 */
async function sendSlack(
  webhookUrl: string,
  message: SlackMessage
): Promise<void> {
  try {
    const res = await fetch(webhookUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(message),
    });
    if (!res.ok) {
      console.error(
        `[notify] Slack webhook failed: ${res.status} ${res.statusText}`
      );
    }
  } catch (err) {
    console.error("[notify] Slack webhook error:", err);
  }
}

function missionUrl(missionId: string): string {
  const base = process.env.APP_URL || "http://localhost:3000";
  return `${base}/missions/${missionId}`;
}

export async function notifyEscalation(
  mission: MissionState,
  requestDescription: string
): Promise<void> {
  if (!mission.slackWebhook) return;

  await sendSlack(mission.slackWebhook, {
    text: `🔴 Research mission "${mission.objective}" needs your input: ${requestDescription}`,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `🔴 *Research mission needs your input*\n\n*Mission:* ${mission.objective}\n*Request:* ${requestDescription}\n\n<${missionUrl(mission.id)}|Respond now →>`,
        },
      },
    ],
  });
}

export async function notifyCheckpoint(mission: MissionState): Promise<void> {
  if (!mission.slackWebhook) return;

  await sendSlack(mission.slackWebhook, {
    text: `🟡 Research mission "${mission.objective}" reached a checkpoint. Review and approve to continue.`,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `🟡 *Research mission checkpoint*\n\n*Mission:* ${mission.objective}\nThe agent has completed a research phase and is waiting for your approval to continue.\n\n<${missionUrl(mission.id)}|Review and approve →>`,
        },
      },
    ],
  });
}

export async function notifyCompleted(mission: MissionState): Promise<void> {
  if (!mission.slackWebhook) return;

  await sendSlack(mission.slackWebhook, {
    text: `🟢 Research mission "${mission.objective}" complete. View report.`,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `🟢 *Research mission complete!*\n\n*Mission:* ${mission.objective}\nYour research report is ready.\n\n<${missionUrl(mission.id)}|View report →>`,
        },
      },
    ],
  });
}

export async function notifyStalled(mission: MissionState): Promise<void> {
  if (!mission.slackWebhook) return;

  await sendSlack(mission.slackWebhook, {
    text: `⚠️ Research mission "${mission.objective}" may be stalled. No new findings for 30+ minutes.`,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `⚠️ *Research mission may be stalled*\n\n*Mission:* ${mission.objective}\nNo new artifacts detected for 30+ minutes. The agent process is still running.\n\n<${missionUrl(mission.id)}|Check status →>`,
        },
      },
    ],
  });
}

export async function notifyFailed(
  mission: MissionState,
  reason: string
): Promise<void> {
  if (!mission.slackWebhook) return;

  await sendSlack(mission.slackWebhook, {
    text: `🔴 Research mission "${mission.objective}" failed. You can relaunch from saved state.`,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `🔴 *Research mission failed*\n\n*Mission:* ${mission.objective}\n*Reason:* ${reason}\n\nYour research state is preserved. You can relaunch from where it left off.\n\n<${missionUrl(mission.id)}|Relaunch →>`,
        },
      },
    ],
  });
}
