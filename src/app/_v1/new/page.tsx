"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";

type Template = "search-relevance" | "system-investigation";
type AutonomyLevel = "full" | "checkpoint";

const templates: Array<{
  id: Template;
  name: string;
  description: string;
}> = [
  {
    id: "search-relevance",
    name: "Search / Relevance",
    description:
      "Improve search quality, relevance ranking, or retrieval performance",
  },
  {
    id: "system-investigation",
    name: "System Investigation",
    description:
      "Investigate a complex technical issue, architecture decision, or system behavior",
  },
];

export default function NewMission() {
  const router = useRouter();
  const [template, setTemplate] = useState<Template>("search-relevance");
  const [objective, setObjective] = useState("");
  const [context, setContext] = useState("");
  const [repository, setRepository] = useState("");
  const [successMetric, setSuccessMetric] = useState("");
  const [autonomyLevel, setAutonomyLevel] = useState<AutonomyLevel>("full");
  const [maxRuntime, setMaxRuntime] = useState("48h");
  const [slackWebhook, setSlackWebhook] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);

    try {
      const res = await fetch("/api/missions", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          template,
          objective,
          context,
          repository: repository || undefined,
          successMetric,
          autonomyLevel,
          maxRuntime,
          slackWebhook: slackWebhook || undefined,
        }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to create mission");
      }

      const data = await res.json();
      router.push(`/missions/${data.mission.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create mission");
      setSubmitting(false);
    }
  }

  return (
    <div className="min-h-screen bg-[#fafafa]">
      <div className="max-w-2xl mx-auto px-6 py-12">
        <Link
          href="/"
          className="text-sm text-[#737373] hover:text-[#0a0a0a] mb-6 inline-block"
        >
          &larr; Back to missions
        </Link>

        <h1 className="text-2xl font-semibold text-[#0a0a0a] tracking-tight mb-8">
          New Research Mission
        </h1>

        <form onSubmit={handleSubmit} className="space-y-6">
          {/* Template selector */}
          <div>
            <label className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-3">
              Template
            </label>
            <div className="grid grid-cols-2 gap-3">
              {templates.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => setTemplate(t.id)}
                  className={`p-4 rounded-md border text-left transition-colors ${
                    template === t.id
                      ? "border-[#0a0a0a] bg-white"
                      : "border-[#e5e5e5] bg-white hover:border-[#d4d4d4]"
                  }`}
                >
                  <p className="text-sm font-medium text-[#0a0a0a]">
                    {t.name}
                  </p>
                  <p className="text-xs text-[#737373] mt-1">
                    {t.description}
                  </p>
                </button>
              ))}
            </div>
          </div>

          {/* Objective */}
          <div>
            <label
              htmlFor="objective"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Research Objective *
            </label>
            <textarea
              id="objective"
              rows={3}
              value={objective}
              onChange={(e) => setObjective(e.target.value)}
              placeholder="Describe the technical problem you want investigated..."
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] placeholder-[#a3a3a3] bg-white focus:outline-none focus:border-[#0a0a0a]"
              required
              minLength={50}
            />
            <p className="text-xs text-[#a3a3a3] mt-1">
              Minimum 50 characters
            </p>
          </div>

          {/* Context */}
          <div>
            <label
              htmlFor="context"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Context & Baseline *
            </label>
            <textarea
              id="context"
              rows={3}
              value={context}
              onChange={(e) => setContext(e.target.value)}
              placeholder="Describe your current system and what you've tried..."
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] placeholder-[#a3a3a3] bg-white focus:outline-none focus:border-[#0a0a0a]"
              required
              minLength={30}
            />
          </div>

          {/* Repository */}
          <div>
            <label
              htmlFor="repository"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Repository
            </label>
            <input
              id="repository"
              type="text"
              value={repository}
              onChange={(e) => setRepository(e.target.value)}
              placeholder="github.com/org/repo or local path"
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] placeholder-[#a3a3a3] bg-white focus:outline-none focus:border-[#0a0a0a]"
            />
          </div>

          {/* Success Metric */}
          <div>
            <label
              htmlFor="successMetric"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Success Metric *
            </label>
            <input
              id="successMetric"
              type="text"
              value={successMetric}
              onChange={(e) => setSuccessMetric(e.target.value)}
              placeholder="e.g., Recall@10 improvement ≥ 5%"
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] placeholder-[#a3a3a3] bg-white focus:outline-none focus:border-[#0a0a0a]"
              required
            />
          </div>

          {/* Autonomy Level */}
          <div>
            <label className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-3">
              Autonomy Level
            </label>
            <div className="space-y-2">
              {[
                {
                  value: "full" as const,
                  label: "Full autonomy",
                  desc: "Agent runs continuously, escalates only when blocked",
                },
                {
                  value: "checkpoint" as const,
                  label: "Checkpoint mode",
                  desc: "Agent pauses after each phase for your approval",
                },
              ].map((opt) => (
                <label
                  key={opt.value}
                  className={`flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors ${
                    autonomyLevel === opt.value
                      ? "border-[#0a0a0a] bg-white"
                      : "border-[#e5e5e5] bg-white hover:border-[#d4d4d4]"
                  }`}
                >
                  <input
                    type="radio"
                    name="autonomy"
                    value={opt.value}
                    checked={autonomyLevel === opt.value}
                    onChange={() => setAutonomyLevel(opt.value)}
                    className="mt-0.5"
                  />
                  <div>
                    <p className="text-sm font-medium text-[#0a0a0a]">
                      {opt.label}
                    </p>
                    <p className="text-xs text-[#737373]">{opt.desc}</p>
                  </div>
                </label>
              ))}
            </div>
          </div>

          {/* Max Runtime */}
          <div>
            <label
              htmlFor="maxRuntime"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Max Runtime
            </label>
            <select
              id="maxRuntime"
              value={maxRuntime}
              onChange={(e) => setMaxRuntime(e.target.value)}
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] bg-white focus:outline-none focus:border-[#0a0a0a]"
            >
              <option value="12h">12 hours</option>
              <option value="24h">24 hours</option>
              <option value="48h">48 hours (default)</option>
              <option value="72h">72 hours</option>
              <option value="unlimited">Unlimited</option>
            </select>
          </div>

          {/* Slack Webhook */}
          <div>
            <label
              htmlFor="slack"
              className="block text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2"
            >
              Slack Webhook URL
            </label>
            <input
              id="slack"
              type="url"
              value={slackWebhook}
              onChange={(e) => setSlackWebhook(e.target.value)}
              placeholder="https://hooks.slack.com/services/..."
              className="w-full px-3 py-2 border border-[#e5e5e5] rounded-md text-sm text-[#0a0a0a] placeholder-[#a3a3a3] bg-white focus:outline-none focus:border-[#0a0a0a]"
            />
            <p className="text-xs text-[#a3a3a3] mt-1">
              Get notified when the agent needs input or completes
            </p>
          </div>

          {/* Estimate display */}
          <div className="bg-white border border-[#e5e5e5] rounded-md p-4">
            <p className="text-xs font-semibold text-[#737373] uppercase tracking-wider mb-2">
              Estimated
            </p>
            <div className="flex gap-6">
              <div>
                <p className="text-sm text-[#0a0a0a] font-medium">
                  {template === "search-relevance" ? "$30–$150" : "$20–$100"}
                </p>
                <p className="text-xs text-[#737373]">API token cost</p>
              </div>
              <div>
                <p className="text-sm text-[#0a0a0a] font-medium">
                  {template === "search-relevance"
                    ? "12–48 hours"
                    : "8–36 hours"}
                </p>
                <p className="text-xs text-[#737373]">Duration</p>
              </div>
            </div>
          </div>

          {error && (
            <div className="bg-red-50 border border-red-200 rounded-md p-3">
              <p className="text-sm text-red-700">{error}</p>
            </div>
          )}

          <button
            type="submit"
            disabled={submitting}
            className="w-full py-3 bg-[#0a0a0a] text-white text-sm font-medium rounded-md hover:bg-[#1a1a1a] transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {submitting ? "Launching..." : "Launch Mission →"}
          </button>
        </form>
      </div>
    </div>
  );
}

