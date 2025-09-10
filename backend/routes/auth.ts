import express from 'express';
import { KiteConnect } from 'kiteconnect';
import { z } from 'zod';

const router = express.Router();

// Validation schemas
const loginSchema = z.object({
  apiKey: z.string().min(1, 'API Key is required'),
  apiSecret: z.string().min(1, 'API Secret is required'),
  requestToken: z.string().min(1, 'Request Token is required')
});

// Store active sessions (in production, use Redis or database)
const activeSessions = new Map<string, { accessToken: string; kiteConnect: any }>();

/**
 * Generate login URL for Kite Connect authentication
 */
router.post('/login-url', (req, res) => {
  try {
    const { apiKey } = req.body;
    
    if (!apiKey) {
      return res.status(400).json({ error: 'API Key is required' });
    }

    const kc = new KiteConnect({ api_key: apiKey });
    const loginURL = kc.getLoginURL();
    
    res.json({ loginURL });
  } catch (error) {
    console.error('Error generating login URL:', error);
    res.status(500).json({ error: 'Failed to generate login URL' });
  }
});

/**
 * Generate access token from request token
 */
router.post('/generate-token', async (req, res) => {
  try {
    const validation = loginSchema.safeParse(req.body);
    
    if (!validation.success) {
      return res.status(400).json({ 
        error: 'Validation failed', 
        details: validation.error.errors 
      });
    }

    const { apiKey, apiSecret, requestToken } = validation.data;
    
    const kc = new KiteConnect({ api_key: apiKey });
    const response = await kc.generateSession(requestToken, apiSecret);
    
    // Store session
    const sessionId = `session_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    activeSessions.set(sessionId, {
      accessToken: response.access_token,
      kiteConnect: kc
    });
    
    res.json({ 
      sessionId,
      accessToken: response.access_token,
      message: 'Authentication successful'
    });
  } catch (error: any) {
    console.error('Error generating access token:', error);
    res.status(400).json({ 
      error: 'Authentication failed', 
      message: error.message || 'Invalid credentials or request token'
    });
  }
});

/**
 * Get user profile
 */
router.get('/profile/:sessionId', async (req, res) => {
  try {
    const { sessionId } = req.params;
    const session = activeSessions.get(sessionId);
    
    if (!session) {
      return res.status(404).json({ error: 'Session not found' });
    }

    const profile = await session.kiteConnect.getProfile();
    res.json(profile);
  } catch (error: any) {
    console.error('Error fetching profile:', error);
    res.status(500).json({ 
      error: 'Failed to fetch profile', 
      message: error.message 
    });
  }
});

/**
 * Logout and clear session
 */
router.post('/logout/:sessionId', (req, res) => {
  const { sessionId } = req.params;
  activeSessions.delete(sessionId);
  res.json({ message: 'Logged out successfully' });
});

export { router as authRoutes, activeSessions };
