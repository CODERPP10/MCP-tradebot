import { KiteConnect } from "kiteconnect";

const apiKey = "lq8i0m3my25elb9a";
const apiSecret = "ghlomv5fpzjk3xyuc16cxg2j7wzghqzp";
// const requestToken = "A2Gi8BcmHTk3mS4jnXrvTYzHEf03fApf";
let accessToken = "n3QatWw9sBvpcq6GCcdCa3oVRKjRRWj6";

const kc = new KiteConnect({ api_key: apiKey });

// console.log(kc.getLoginURL());

export async function placeOrder(tradingsymbol: string, quantity: number, type: "BUY" | "SELL") {
  try {
    // kc.setAccessToken(accessToken)
    const order = await kc.placeOrder("amo", {
      exchange: "NSE",
      tradingsymbol: tradingsymbol,
      transaction_type: type,
      quantity: quantity,
      product: "CNC",
      order_type: "MARKET"
    });
  console.log("Order placed:", order);
} catch (err) {
  console.error("Error placing order:", err);
}
}

// async function init() {
//   await generateSession();
// }

// async function generateSession() {
//   try {
//     const response = await kc.generateSession(requestToken, apiSecret);
//     kc.setAccessToken(response.access_token);
//     console.log("Session generated:", response);
//     console.log("Access token:", response.access_token);
//   } catch (err) {
//     console.error("Error generating session:", err);
//   }
// }

// init();