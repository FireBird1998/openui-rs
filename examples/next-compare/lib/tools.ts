// In-memory answers for the tools the docs/* programs call (Query and
// Mutation), one set per pane so neither pane sees the other's writes.
type Ticket = { id: string; title: string; priority: string };

export function makeTools() {
  let next = 4;
  const rows: Ticket[] = [
    { id: "1", title: "Login page times out", priority: "high" },
    { id: "2", title: "Typo on the pricing page", priority: "low" },
    { id: "3", title: "Export to CSV", priority: "medium" },
  ];
  return {
    list_tickets: async () => ({ rows: rows.map((r) => ({ ...r })) }),
    create_ticket: async (args: Record<string, unknown>) => {
      const ticket = { id: String(next++), title: String(args.title ?? ""), priority: String(args.priority ?? "medium") };
      rows.push(ticket);
      return ticket;
    },
    update_ticket: async (args: Record<string, unknown>) => {
      const ticket = rows.find((r) => r.id === args.id);
      if (ticket) Object.assign(ticket, { title: String(args.title ?? ticket.title), priority: String(args.priority ?? ticket.priority) });
      return ticket ?? null;
    },
  };
}
