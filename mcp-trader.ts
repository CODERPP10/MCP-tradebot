import { placeOrder } from "./trade";
import { McpServer, ResourceTemplate } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

// Create an MCP server
const server = new McpServer({
  name: "Demo",
  version: "1.0.0"
});

// buy a stock
server.tool(
  "buy_stock",
  { stock: z.string(), qty: z.number() },
  async ({ stock, qty }) => { 
    // console.log(`Buying ${qty} shares of ${stock}`);
    placeOrder(stock, qty, "BUY"); 
    return { content: [{ type: "text", text: `Bought ${qty} shares of ${stock}` }] };
  }
);

// sell a stock
server.tool(
  "sell_stock",
  { stock: z.string(), qty: z.number() },
  async ({ stock, qty }) => { 
    // console.log(`Selling ${qty} shares of ${stock}`);
    placeOrder(stock, qty, "SELL");
    return { content: [{ type: "text", text: `Sold ${qty} shares of ${stock}` }] };
  }
);

// Start receiving messages on stdin and sending messages on stdout
const transport = new StdioServerTransport();
await server.connect(transport);