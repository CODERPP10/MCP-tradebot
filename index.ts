// import { placeOrder } from "./trade";
import { KiteConnect } from "kiteconnect"; 


const apiKey = "lq8i0m3my25elb9a";
const apiSecret = "ghlomv5fpzjk3xyuc16cxg2j7wzghqzp";
const requestToken = "bZ5coYltgF2I008o8R3RGBg9hPAV3a7R";
let accessToken = "GOaszQQgFZNkSC7xjCM5RGzMjHSFmv8T";

const kc = new KiteConnect({ api_key: apiKey });

// placeOrder("ONGC", 1, "SELL");
// placeOrder("ONGC", 1, "BUY");

async function init() {
      try {
        kc.setAccessToken(accessToken);
        // await profile();
        await placeOrder();
      } catch (err) {
        console.error(err);
      }
    }

async function placeOrder() {
    const loginURL = kc.getLoginURL();
    console.log(loginURL);

    const response = await fetch(loginURL);
}


    
// Initialize the API calls
init();

