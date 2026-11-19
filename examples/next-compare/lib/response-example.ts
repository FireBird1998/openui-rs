// Both the React Renderer and the wasm session receive this exact response.
export const response = [
  "]]>openui:content?name=Availability",
  'status=Query("availability",{},"Checking")',
  "root=TextContent(status)",
  ']]>openui:scripts {"count":1}',
  '[{"name":"availability","code":"cloud implementation"}]',
  "]]>openui:end",
].join("\n");

export const tools = { availability: async () => "Available" };
