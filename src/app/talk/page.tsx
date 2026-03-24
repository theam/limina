"use client";

import { useEffect, useState, useCallback, useRef } from "react";
import Nav from "../../components/nav";

interface Message {
  id: string;
  role: "user" | "assistant";
  content: string;
}

export default function TalkPage() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Load session ID from localStorage
  useEffect(() => {
    const saved = localStorage.getItem("limina-chat-session");
    if (saved) setSessionId(saved);
  }, []);

  // Auto-scroll on new messages
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const sendMessage = useCallback(
    async (text: string) => {
      if (!text.trim() || isStreaming) return;

      const userMsg: Message = {
        id: `user-${Date.now()}`,
        role: "user",
        content: text.trim(),
      };

      const assistantId = `assistant-${Date.now()}`;
      const assistantMsg: Message = {
        id: assistantId,
        role: "assistant",
        content: "",
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setInput("");
      setIsStreaming(true);

      try {
        const res = await fetch("/api/chat", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ message: text.trim(), sessionId }),
        });

        if (!res.ok || !res.body) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, content: "Failed to connect. Is the agent running?" }
                : m
            )
          );
          setIsStreaming(false);
          return;
        }

        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";

        while (true) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() || "";

          for (const line of lines) {
            if (!line.trim()) continue;
            try {
              const chunk = JSON.parse(line);

              if (chunk.type === "session" && chunk.sessionId) {
                setSessionId(chunk.sessionId);
                localStorage.setItem("limina-chat-session", chunk.sessionId);
              }

              if (chunk.type === "text" && chunk.text) {
                setMessages((prev) =>
                  prev.map((m) =>
                    m.id === assistantId
                      ? { ...m, content: m.content + chunk.text }
                      : m
                  )
                );
              }

              if (chunk.type === "done" && chunk.sessionId) {
                setSessionId(chunk.sessionId);
                localStorage.setItem("limina-chat-session", chunk.sessionId);
              }

              if (chunk.type === "error") {
                setMessages((prev) =>
                  prev.map((m) =>
                    m.id === assistantId
                      ? { ...m, content: chunk.error || "An error occurred." }
                      : m
                  )
                );
              }
            } catch {
              // Skip malformed JSON
            }
          }
        }
      } catch {
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantId
              ? { ...m, content: "Connection lost. Try again." }
              : m
          )
        );
      } finally {
        setIsStreaming(false);
        inputRef.current?.focus();
      }
    },
    [isStreaming, sessionId]
  );

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    sendMessage(input);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage(input);
    }
  };

  return (
    <div
      style={{
        minHeight: "100vh",
        marginLeft: 64,
        backgroundColor: "#f4f4f4",
        fontFamily: "'IBM Plex Sans', sans-serif",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Nav activePath="/talk" />

      {/* Messages area */}
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflowY: "auto",
          padding: "24px 0",
        }}
      >
        <div style={{ maxWidth: 720, margin: "0 auto", padding: "0 24px" }}>
          {messages.length === 0 ? (
            <div
              style={{
                textAlign: "center",
                paddingTop: 120,
              }}
            >
              <h1
                style={{
                  fontSize: 20,
                  fontWeight: 600,
                  color: "#161616",
                  marginBottom: 8,
                }}
              >
                Talk to Limina
              </h1>
              <p
                style={{
                  fontSize: 14,
                  color: "#525252",
                  lineHeight: 1.6,
                  maxWidth: 420,
                  margin: "0 auto",
                }}
              >
                Ask anything about the research. I can see everything the agent
                is working on — hypotheses, experiments, findings, and
                decisions. I can also send directives to change its course.
              </p>
            </div>
          ) : (
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 20,
              }}
            >
              {messages.map((msg) => (
                <div
                  key={msg.id}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    alignItems:
                      msg.role === "user" ? "flex-end" : "flex-start",
                  }}
                >
                  <div
                    style={{
                      maxWidth: "85%",
                      padding: "12px 16px",
                      borderRadius: 8,
                      backgroundColor:
                        msg.role === "user" ? "#0f62fe" : "#ffffff",
                      color: msg.role === "user" ? "#ffffff" : "#161616",
                      border:
                        msg.role === "assistant"
                          ? "1px solid #e0e0e0"
                          : "none",
                      fontSize: 14,
                      lineHeight: 1.6,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                    }}
                  >
                    {msg.content ||
                      (msg.role === "assistant" && isStreaming ? (
                        <span
                          style={{
                            color: "#8d8d8d",
                            fontStyle: "italic",
                          }}
                        >
                          Thinking...
                        </span>
                      ) : null)}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Input area */}
      <div
        style={{
          borderTop: "1px solid #e0e0e0",
          backgroundColor: "#ffffff",
          padding: "16px 24px",
        }}
      >
        <form
          onSubmit={handleSubmit}
          style={{
            maxWidth: 720,
            margin: "0 auto",
            display: "flex",
            gap: 8,
          }}
        >
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Ask about the research, or tell the agent what to do..."
            rows={1}
            disabled={isStreaming}
            style={{
              flex: 1,
              padding: "10px 14px",
              border: "1px solid #e0e0e0",
              borderRadius: 6,
              fontSize: 14,
              fontFamily: "'IBM Plex Sans', sans-serif",
              backgroundColor: isStreaming ? "#f4f4f4" : "#ffffff",
              color: "#161616",
              resize: "none",
              outline: "none",
              lineHeight: 1.5,
            }}
          />
          <button
            type="submit"
            disabled={isStreaming || !input.trim()}
            style={{
              padding: "10px 20px",
              backgroundColor:
                isStreaming || !input.trim() ? "#c6c6c6" : "#0f62fe",
              color: "#ffffff",
              border: "none",
              borderRadius: 6,
              fontSize: 14,
              fontWeight: 500,
              fontFamily: "'IBM Plex Sans', sans-serif",
              cursor:
                isStreaming || !input.trim() ? "not-allowed" : "pointer",
              flexShrink: 0,
            }}
          >
            Send
          </button>
        </form>
      </div>
    </div>
  );
}
