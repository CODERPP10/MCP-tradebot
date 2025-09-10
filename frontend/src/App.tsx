import React, { useState, useEffect } from 'react';
import { AuthForm } from './components/AuthForm';
import { TradingInterface } from './components/TradingInterface';
import { Portfolio } from './components/Portfolio';
import { LogOut } from 'lucide-react';

interface User {
  sessionId: string;
  accessToken: string;
  profile?: any;
}

function App() {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Check for existing session on app load
  useEffect(() => {
    const savedSession = localStorage.getItem('tradingSession');
    if (savedSession) {
      try {
        const session = JSON.parse(savedSession);
        setUser(session);
      } catch (err) {
        localStorage.removeItem('tradingSession');
      }
    }
  }, []);

  const handleLogin = (sessionData: User) => {
    setUser(sessionData);
    localStorage.setItem('tradingSession', JSON.stringify(sessionData));
    setError(null);
  };

  const handleLogout = () => {
    if (user?.sessionId) {
      // Call logout API
      fetch(`/api/auth/logout/${user.sessionId}`, {
        method: 'POST',
      }).catch(console.error);
    }
    
    setUser(null);
    localStorage.removeItem('tradingSession');
    setError(null);
  };

  const handleError = (errorMessage: string) => {
    setError(errorMessage);
  };

  if (loading) {
    return (
      <div className="container">
        <div className="card" style={{ textAlign: 'center', padding: '40px' }}>
          <div style={{ fontSize: '18px' }}>Loading...</div>
        </div>
      </div>
    );
  }

  return (
    <div className="container">
      <div className="header">
        <h1>🚀 MCP TradeBot</h1>
        <p>Professional Trading Interface with Kite Connect</p>
      </div>

      {error && (
        <div className="card" style={{ background: '#fef2f2', border: '1px solid #fecaca' }}>
          <div className="error-message" style={{ fontSize: '16px' }}>
            {error}
          </div>
        </div>
      )}

      {!user ? (
        <AuthForm onLogin={handleLogin} onError={handleError} />
      ) : (
        <div>
          <div className="card" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <div>
              <div className="status-indicator status-connected">
                <div style={{ width: '8px', height: '8px', background: '#10b981', borderRadius: '50%' }}></div>
                Connected to Kite
              </div>
            </div>
            <button 
              className="btn btn-danger" 
              onClick={handleLogout}
              style={{ display: 'flex', alignItems: 'center', gap: '8px' }}
            >
              <LogOut size={16} />
              Logout
            </button>
          </div>

          <div className="grid grid-2">
            <TradingInterface 
              sessionId={user.sessionId} 
              onError={handleError}
            />
            <Portfolio 
              sessionId={user.sessionId} 
              onError={handleError}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
