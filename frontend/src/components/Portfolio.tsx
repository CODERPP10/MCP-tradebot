import React, { useState, useEffect } from 'react';
import { Portfolio as PortfolioIcon, RefreshCw, TrendingUp, TrendingDown } from 'lucide-react';

interface PortfolioProps {
  sessionId: string;
  onError: (error: string) => void;
}

interface PortfolioData {
  holdings: any[];
  positions: any[];
  orders: any[];
}

export const Portfolio: React.FC<PortfolioProps> = ({ sessionId, onError }) => {
  const [data, setData] = useState<PortfolioData>({ holdings: [], positions: [], orders: [] });
  const [loading, setLoading] = useState(false);
  const [activeTab, setActiveTab] = useState<'holdings' | 'positions' | 'orders'>('holdings');

  const fetchData = async () => {
    setLoading(true);
    try {
      const [holdingsRes, positionsRes, ordersRes] = await Promise.all([
        fetch(`/api/trade/holdings/${sessionId}`),
        fetch(`/api/trade/positions/${sessionId}`),
        fetch(`/api/trade/orders/${sessionId}`)
      ]);

      const holdings = holdingsRes.ok ? await holdingsRes.json() : [];
      const positions = positionsRes.ok ? await positionsRes.json() : [];
      const orders = ordersRes.ok ? await ordersRes.json() : [];

      setData({ holdings, positions, orders });
    } catch (error: any) {
      onError('Failed to fetch portfolio data');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
  }, [sessionId]);

  const formatCurrency = (value: number) => {
    return new Intl.NumberFormat('en-IN', {
      style: 'currency',
      currency: 'INR',
    }).format(value);
  };

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString('en-IN', {
      day: '2-digit',
      month: '2-digit',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  const renderHoldings = () => (
    <div>
      {data.holdings.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '20px', color: '#6b7280' }}>
          No holdings found
        </div>
      ) : (
        <div style={{ maxHeight: '400px', overflowY: 'auto' }}>
          {data.holdings.map((holding, index) => (
            <div key={index} style={{ 
              padding: '12px', 
              border: '1px solid #e5e7eb', 
              borderRadius: '8px', 
              marginBottom: '8px',
              background: '#f9fafb'
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <div>
                  <div style={{ fontWeight: '600', fontSize: '16px' }}>{holding.tradingsymbol}</div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>{holding.exchange}</div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  <div style={{ fontWeight: '600' }}>{holding.quantity} shares</div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>
                    {formatCurrency(holding.average_price)}
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );

  const renderPositions = () => (
    <div>
      {data.positions.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '20px', color: '#6b7280' }}>
          No open positions
        </div>
      ) : (
        <div style={{ maxHeight: '400px', overflowY: 'auto' }}>
          {data.positions.map((position, index) => (
            <div key={index} style={{ 
              padding: '12px', 
              border: '1px solid #e5e7eb', 
              borderRadius: '8px', 
              marginBottom: '8px',
              background: '#f9fafb'
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <div>
                  <div style={{ fontWeight: '600', fontSize: '16px' }}>{position.tradingsymbol}</div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>{position.exchange}</div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  <div style={{ 
                    fontWeight: '600', 
                    color: position.pnl >= 0 ? '#10b981' : '#ef4444',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px'
                  }}>
                    {position.pnl >= 0 ? <TrendingUp size={16} /> : <TrendingDown size={16} />}
                    {formatCurrency(position.pnl)}
                  </div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>
                    {position.quantity} shares
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );

  const renderOrders = () => (
    <div>
      {data.orders.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '20px', color: '#6b7280' }}>
          No orders found
        </div>
      ) : (
        <div style={{ maxHeight: '400px', overflowY: 'auto' }}>
          {data.orders.slice(0, 10).map((order, index) => (
            <div key={index} style={{ 
              padding: '12px', 
              border: '1px solid #e5e7eb', 
              borderRadius: '8px', 
              marginBottom: '8px',
              background: '#f9fafb'
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <div>
                  <div style={{ fontWeight: '600', fontSize: '16px' }}>{order.tradingsymbol}</div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>
                    {formatDate(order.order_timestamp)}
                  </div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  <div style={{ 
                    fontWeight: '600',
                    color: order.transaction_type === 'BUY' ? '#10b981' : '#ef4444'
                  }}>
                    {order.transaction_type} {order.quantity}
                  </div>
                  <div style={{ fontSize: '14px', color: '#6b7280' }}>
                    {order.status}
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );

  return (
    <div className="card">
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
        <h2 style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
          <PortfolioIcon size={24} />
          Portfolio
        </h2>
        <button
          className="btn"
          onClick={fetchData}
          disabled={loading}
          style={{ 
            background: '#6b7280', 
            color: 'white',
            display: 'flex',
            alignItems: 'center',
            gap: '8px'
          }}
        >
          <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      <div style={{ display: 'flex', gap: '8px', marginBottom: '20px' }}>
        {(['holdings', 'positions', 'orders'] as const).map((tab) => (
          <button
            key={tab}
            className="btn"
            onClick={() => setActiveTab(tab)}
            style={{
              background: activeTab === tab ? '#3b82f6' : '#e5e7eb',
              color: activeTab === tab ? 'white' : '#374151',
              textTransform: 'capitalize'
            }}
          >
            {tab}
          </button>
        ))}
      </div>

      {loading ? (
        <div style={{ textAlign: 'center', padding: '40px' }}>
          <div style={{ fontSize: '18px' }}>Loading...</div>
        </div>
      ) : (
        <>
          {activeTab === 'holdings' && renderHoldings()}
          {activeTab === 'positions' && renderPositions()}
          {activeTab === 'orders' && renderOrders()}
        </>
      )}
    </div>
  );
};
