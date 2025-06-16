import { KiteConnect } from "kiteconnect";

const apiKey = "lq8i0m3my25elb9a";
const apiSecret = "ghlomv5fpzjk3xyuc16cxg2j7wzghqzp";
// const requestToken = "iaku0m3biz4jqvOc0Ct7XWKPg6EdAB80";
let accessToken = "n3QatWw9sBvpcq6GCcdCa3oVRKjRRWj6";
const kc = new KiteConnect({ api_key: apiKey });

console.log(kc.getLoginURL());

async function init() {
  try {
    // await generateSession();
    kc.setAccessToken(accessToken);
    await placeOrder();
  } catch (err) {
    console.error(err);
  }
}

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

async function placeOrder() {
  try {
    const order = await kc.placeOrder("regular", {
        exchange: "NSE",
        tradingsymbol: "ONGC",
        transaction_type: "SELL",
        quantity: 1,
        product: "CNC",
        order_type: "MARKET"
      });
    console.log("Order placed:", order);
  } catch (err) {
    console.error("Error placing order:", err);
  }
}
// Initialize the API calls
init();