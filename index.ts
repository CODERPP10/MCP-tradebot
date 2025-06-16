// import { placeOrder } from "./trade";
import { McpServer, ResourceTemplate } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { placeOrder } from "./trade";

// Create an MCP server
const server = new McpServer({
  name: "Demo",
  version: "1.0.0"
});

// Add an addition tool
server.tool("add",
  { a: z.number(), b: z.number() },
  async ({ a, b }) => ({
    content: [{ type: "text", text: String(a + b) }]
  })
);

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

// Factor an factorial tool
server.tool("factorial",
  { a: z.number() },
  async ({ a }) => {
    let result = 1;
    for (let i = 1; i <= a; i++) {
      result *= i;
    }
    return { content: [{ type: "text", text: String(result) }] };
  }
);

// Start receiving messages on stdin and sending messages on stdout
const transport = new StdioServerTransport();
await server.connect(transport);