import { KiteConnect } from "kiteconnect";
import dotenv from 'dotenv';

// Load environment variables
dotenv.config();

// Get credentials from environment variables
const apiKey = process.env.KITE_API_KEY;
const apiSecret = process.env.KITE_API_SECRET;

if (!apiKey || !apiSecret) {
  throw new Error('KITE_API_KEY and KITE_API_SECRET must be set in environment variables');
}

const kc = new KiteConnect({ api_key: apiKey });

/**
 * Place a trading order using Kite Connect API
 * @param tradingsymbol - The trading symbol of the instrument
 * @param quantity - Number of shares to trade
 * @param type - BUY or SELL
 * @param accessToken - Valid access token for authentication
 */
export async function placeOrder(
  tradingsymbol: string, 
  quantity: number, 
  type: "BUY" | "SELL",
  accessToken: string
): Promise<any> {
  try {
    if (!accessToken) {
      throw new Error('Access token is required for trading');
    }

    kc.setAccessToken(accessToken);
    
    const order = await kc.placeOrder("amo", {
      exchange: "NSE",
      tradingsymbol: tradingsymbol,
      transaction_type: type,
      quantity: quantity,
      product: "CNC",
      order_type: "MARKET"
    });
    
    console.log("Order placed:", order);
    return order;
  } catch (err) {
    console.error("Error placing order:", err);
    throw err;
  }
}

/**
 * Generate login URL for Kite Connect authentication
 */
export function getLoginURL(): string {
  return kc.getLoginURL();
}

/**
 * Generate access token from request token
 * @param requestToken - Request token obtained from login redirect
 */
export async function generateAccessToken(requestToken: string): Promise<string> {
  try {
    const response = await kc.generateSession(requestToken, apiSecret);
    return response.access_token;
  } catch (err) {
    console.error("Error generating access token:", err);
    throw err;
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