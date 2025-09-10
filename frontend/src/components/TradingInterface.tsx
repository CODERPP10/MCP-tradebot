import React, { useState } from 'react';
import { TrendingUp, TrendingDown, Send } from 'lucide-react';

interface TradingInterfaceProps {
  sessionId: string;
  onError: (error: string) => void;
}

export const TradingInterface: React.FC<TradingInterfaceProps> = ({ sessionId, onError }) => {
  const [symbol, setSymbol] = useState('');
  const [quantity, setQuantity] = useState('');
  const [orderType, setOrderType] = useState<'BUY' | 'SELL'>('BUY');
  const [loading, setLoading] = useState(false);
  const [lastOrder, setLastOrder] = useState<any>(null);

  const handlePlaceOrder = async () => {
    if (!symbol.trim() || !quantity.trim()) {
      onError('Symbol and quantity are required');
      return;
    }

    const qty = parseInt(quantity);
    if (isNaN(qty) || qty <= 0) {
      onError('Quantity must be a positive number');
      return;
    }

    setLoading(true);
    try {
      const response = await fetch('/api/trade/place-order', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          sessionId,
          tradingsymbol: symbol.toUpperCase(),
          quantity: qty,
          type: orderType,
        }),
      });

      const data = await response.json();

      if (!response.ok) {
        throw new Error(data.error || 'Failed to place order');
      }

      setLastOrder(data);
      setSymbol('');
      setQuantity('');
    } catch (error: any) {
      onError(error.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="card">
      <h2 style={{ marginBottom: '24px', display: 'flex', alignItems: 'center', gap: '12px' }}>
        {orderType === 'BUY' ? <TrendingUp size={24} color="#10b981" /> : <TrendingDown size={24} color="#ef4444" />}
        Place Order
      </h2>

      <div className="trading-form">
        <div className="form-group">
          <label className="form-label">Trading Symbol</label>
          <input
            type="text"
            className="form-input"
            value={symbol}
            onChange={(e) => setSymbol(e.target.value.toUpperCase())}
            placeholder="e.g., RELIANCE, TCS, INFY"
            style={{ textTransform: 'uppercase' }}
          />
        </div>

        <div className="form-group">
          <label className="form-label">Quantity</label>
          <input
            type="number"
            className="form-input"
            value={quantity}
            onChange={(e) => setQuantity(e.target.value)}
            placeholder="Number of shares"
            min="1"
          />
        </div>

        <div className="form-group">
          <label className="form-label">Order Type</label>
          <div style={{ display: 'flex', gap: '8px' }}>
            <button
              className={`btn ${orderType === 'BUY' ? 'btn-success' : ''}`}
              onClick={() => setOrderType('BUY')}
              style={{ 
                flex: 1, 
                background: orderType === 'BUY' ? '#10b981' : '#e5e7eb',
                color: orderType === 'BUY' ? 'white' : '#374151'
              }}
            >
              <TrendingUp size={16} />
              BUY
            </button>
            <button
              className={`btn ${orderType === 'SELL' ? 'btn-danger' : ''}`}
              onClick={() => setOrderType('SELL')}
              style={{ 
                flex: 1, 
                background: orderType === 'SELL' ? '#ef4444' : '#e5e7eb',
                color: orderType === 'SELL' ? 'white' : '#374151'
              }}
            >
              <TrendingDown size={16} />
              SELL
            </button>
          </div>
        </div>
      </div>

      <button
        className="btn btn-primary"
        onClick={handlePlaceOrder}
        disabled={loading || !symbol.trim() || !quantity.trim()}
        style={{ width: '100%', marginTop: '16px' }}
      >
        {loading ? (
          'Placing Order...'
        ) : (
          <>
            <Send size={16} />
            Place {orderType} Order
          </>
        )}
      </button>

      {lastOrder && (
        <div style={{ 
          marginTop: '20px', 
          padding: '16px', 
          background: '#f0fdf4', 
          borderRadius: '8px', 
          border: '1px solid #bbf7d0' 
        }}>
          <h3 style={{ color: '#166534', marginBottom: '8px' }}>Order Placed Successfully!</h3>
          <div style={{ fontSize: '14px', color: '#15803d' }}>
            <div><strong>Symbol:</strong> {lastOrder.order?.tradingsymbol}</div>
            <div><strong>Type:</strong> {lastOrder.order?.transaction_type}</div>
            <div><strong>Quantity:</strong> {lastOrder.order?.quantity}</div>
            <div><strong>Order ID:</strong> {lastOrder.order?.order_id}</div>
          </div>
        </div>
      )}
    </div>
  );
};
