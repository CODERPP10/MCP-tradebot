import express from 'express';
import { z } from 'zod';
import { activeSessions } from './auth.js';

const router = express.Router();

// Validation schemas
const placeOrderSchema = z.object({
  sessionId: z.string().min(1, 'Session ID is required'),
  tradingsymbol: z.string().min(1, 'Trading symbol is required'),
  quantity: z.number().positive('Quantity must be positive'),
  type: z.enum(['BUY', 'SELL'], { 
    errorMap: () => ({ message: 'Type must be either BUY or SELL' })
  })
});

/**
 * Place a trading order
 */
router.post('/place-order', async (req, res) => {
  try {
    const validation = placeOrderSchema.safeParse(req.body);
    
    if (!validation.success) {
      return res.status(400).json({ 
        error: 'Validation failed', 
        details: validation.error.errors 
      });
    }

    const { sessionId, tradingsymbol, quantity, type } = validation.data;
    const session = activeSessions.get(sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found. Please login again.' });
    }

    const order = await session.kiteConnect.placeOrder("amo", {
      exchange: "NSE",
      tradingsymbol: tradingsymbol,
      transaction_type: type,
      quantity: quantity,
      product: "CNC",
      order_type: "MARKET"
    });

    res.json({ 
      success: true,
      order,
      message: `${type} order placed for ${quantity} shares of ${tradingsymbol}`
    });
  } catch (error: any) {
    console.error('Error placing order:', error);
    res.status(500).json({ 
      error: 'Failed to place order', 
      message: error.message || 'Unknown error occurred'
    });
  }
});

/**
 * Get order book
 */
router.get('/orders/:sessionId', async (req, res) => {
  try {
    const { sessionId } = req.params;
    const session = activeSessions.get(sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found. Please login again.' });
    }

    const orders = await session.kiteConnect.getOrders();
    res.json(orders);
  } catch (error: any) {
    console.error('Error fetching orders:', error);
    res.status(500).json({ 
      error: 'Failed to fetch orders', 
      message: error.message 
    });
  }
});

/**
 * Get positions
 */
router.get('/positions/:sessionId', async (req, res) => {
  try {
    const { sessionId } = req.params;
    const session = activeSessions.get(sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found. Please login again.' });
    }

    const positions = await session.kiteConnect.getPositions();
    res.json(positions);
  } catch (error: any) {
    console.error('Error fetching positions:', error);
    res.status(500).json({ 
      error: 'Failed to fetch positions', 
      message: error.message 
    });
  }
});

/**
 * Get holdings
 */
router.get('/holdings/:sessionId', async (req, res) => {
  try {
    const { sessionId } = req.params;
    const session = activeSessions.get(sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found. Please login again.' });
    }

    const holdings = await session.kiteConnect.getHoldings();
    res.json(holdings);
  } catch (error: any) {
    console.error('Error fetching holdings:', error);
    res.status(500).json({ 
      error: 'Failed to fetch holdings', 
      message: error.message 
    });
  }
});

export { router as tradeRoutes };
