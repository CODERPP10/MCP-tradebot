import { KiteConnect } from "kiteconnect";

const apiKey = "lq8i0m3my25elb9a";
const apiSecret = "ghlomv5fpzjk3xyuc16cxg2j7wzghqzp";
let accessToken = "GOaszQQgFZNkSC7xjCM5RGzMjHSFmv8T";

const kc = new KiteConnect({ api_key: apiKey });

console.log(kc.getLoginURL());


// async function profile() {
//   try {
//     const profile = await kc.getProfile();
//     console.log("Profile:", profile);
//   } catch (err) {
//     console.error("Error getting profile:", err);
//   }
// }

export async function placeOrder( tradingsymbol: string, quantity: number, type: "BUY" | "SELL" ) {
  try {
    const order = await kc.placeOrder("amo", {
        exchange: "NSE",
        tradingsymbol,
        transaction_type: type,
        quantity,
        product: "CNC",
        order_type: "MARKET"
      });
    console.log("Order placed:", order);
  } 
  catch (err) {
    console.error("Error getting profile:", err);
  }
}
