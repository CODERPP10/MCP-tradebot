# 🚀 MCP TradeBot

A modern, full-stack trading application that integrates with Kite Connect API. Features a React frontend with a professional trading interface and a secure Express.js backend.

## ✨ Features

- **🔐 Secure Authentication**: OAuth-like flow with Kite Connect
- **📊 Trading Interface**: Place BUY/SELL orders with real-time feedback
- **💼 Portfolio Management**: View holdings, positions, and order history
- **🎨 Modern UI**: Responsive design with Tailwind-inspired styling
- **🔒 Security**: Environment-based credential management
- **⚡ Fast**: Built with Bun runtime and Vite for optimal performance

## 🏗️ Architecture

```
├── backend/           # Express.js API server
│   ├── server.ts     # Main server file
│   └── routes/       # API route handlers
├── frontend/         # React frontend application
│   ├── src/
│   │   ├── components/  # React components
│   │   └── App.tsx     # Main app component
│   └── package.json   # Frontend dependencies
├── trade.ts          # Kite Connect integration
└── package.json      # Backend dependencies
```

## 🚀 Quick Start

### Prerequisites

- [Bun](https://bun.sh) installed on your system
- Kite Connect API credentials (API Key & Secret)

### Installation

1. **Clone and install dependencies:**
```bash
bun install
cd frontend && bun install
```

2. **Set up environment variables:**
```bash
cp .env.example .env
```

3. **Configure your Kite Connect credentials in `.env`:**
```env
KITE_API_KEY=your_api_key_here
KITE_API_SECRET=your_api_secret_here
PORT=3001
NODE_ENV=development
FRONTEND_URL=http://localhost:3000
```

### Running the Application

**Option 1: Run both frontend and backend together:**
```bash
bun run dev
```

**Option 2: Run separately:**

Backend (Terminal 1):
```bash
bun run server
```

Frontend (Terminal 2):
```bash
bun run client
```

The application will be available at:
- **Frontend**: http://localhost:3000
- **Backend API**: http://localhost:3001

## 🔧 Usage

### 1. Authentication
1. Open the application in your browser
2. Enter your Kite Connect API Key and Secret
3. Click "Get Login URL" to open Kite Connect login
4. Complete authentication on Kite Connect
5. Copy the request token from the redirect URL
6. Paste the token and complete authentication

### 2. Trading
1. Once authenticated, you'll see the trading interface
2. Enter the trading symbol (e.g., RELIANCE, TCS)
3. Specify the quantity
4. Choose BUY or SELL
5. Click "Place Order" to execute the trade

### 3. Portfolio Management
- View your current holdings
- Check open positions with P&L
- Review recent orders
- Refresh data anytime

## 🛠️ Development

### Project Structure

- **Backend**: Express.js server with TypeScript
- **Frontend**: React with TypeScript and Vite
- **API**: RESTful endpoints for all trading operations
- **Security**: Environment variables and input validation

### Key Files

- `backend/server.ts` - Main Express server
- `backend/routes/auth.ts` - Authentication endpoints
- `backend/routes/trade.ts` - Trading operations
- `frontend/src/App.tsx` - Main React application
- `trade.ts` - Kite Connect integration service

### API Endpoints

**Authentication:**
- `POST /api/auth/login-url` - Get Kite Connect login URL
- `POST /api/auth/generate-token` - Exchange request token for access token
- `GET /api/auth/profile/:sessionId` - Get user profile
- `POST /api/auth/logout/:sessionId` - Logout and clear session

**Trading:**
- `POST /api/trade/place-order` - Place a trading order
- `GET /api/trade/orders/:sessionId` - Get order history
- `GET /api/trade/positions/:sessionId` - Get current positions
- `GET /api/trade/holdings/:sessionId` - Get portfolio holdings

## 🔒 Security Features

- Environment variable management for credentials
- Input validation with Zod schemas
- Session-based authentication
- CORS configuration
- Error handling without information leakage

## 📝 Environment Variables

Create a `.env` file with the following variables:

```env
# Kite Connect API Credentials
KITE_API_KEY=your_api_key_here
KITE_API_SECRET=your_api_secret_here

# Server Configuration
PORT=3001
NODE_ENV=development

# Frontend URL (for CORS)
FRONTEND_URL=http://localhost:3000
```

## 🚨 Important Notes

1. **Never commit your `.env` file** - it contains sensitive credentials
2. **Use the `.env.example`** as a template for required variables
3. **Kite Connect credentials** are required for the application to work
4. **Session management** is currently in-memory (use Redis for production)

## 🐛 Troubleshooting

### Common Issues

1. **"KITE_API_KEY and KITE_API_SECRET must be set"**
   - Ensure your `.env` file exists and contains the required variables

2. **"Session not found"**
   - Re-authenticate with Kite Connect
   - Check if your session has expired

3. **CORS errors**
   - Ensure the backend is running on port 3001
   - Check FRONTEND_URL in your `.env` file

4. **Order placement fails**
   - Verify you have sufficient funds
   - Check if the market is open
   - Ensure the trading symbol is correct

## 📚 Documentation

- [Kite Connect API Documentation](https://kite.trade/docs/connect/v3/)
- [React Documentation](https://react.dev/)
- [Express.js Documentation](https://expressjs.com/)
- [Bun Documentation](https://bun.sh/docs)

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## 📄 License

This project is for educational purposes. Please ensure compliance with Kite Connect's terms of service and applicable regulations.

---

**Built with ❤️ using Bun, React, and TypeScript**
