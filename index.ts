import { KiteConnect } from "kiteconnect";

const apiKey = "lq8i0m3my25elb9a";
const apiSecret = "ghlomv5fpzjk3xyuc16cxg2j7wzghqzp";
const requestToken = "5lTlKYHa2XhTWQthsdAodfF4b6sS87B3";

const kc = new KiteConnect({ api_key: apiKey });

console.log(kc.getLoginURL());

async function init() {
  try {
    await generateSession();
    await placeOrder();
  } catch (err) {
    console.error(err);
  }
}

async function generateSession() {
  try {
    const response = await kc.generateSession(requestToken, apiSecret);
    kc.setAccessToken(response.access_token);
    console.log("Session generated:", response);
  } catch (err) {
    console.error("Error generating session:", err);
  }
}

async function placeOrder() {
  try {
    const order = await kc.placeOrder("amo", {
        exchange: "NSE",
        tradingsymbol: "ONGC",
        transaction_type: "BUY",
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