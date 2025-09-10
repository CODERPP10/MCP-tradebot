import React, { useState } from 'react';
import { ExternalLink, Key, User } from 'lucide-react';

interface AuthFormProps {
  onLogin: (user: { sessionId: string; accessToken: string; profile?: any }) => void;
  onError: (error: string) => void;
}

export const AuthForm: React.FC<AuthFormProps> = ({ onLogin, onError }) => {
  const [apiKey, setApiKey] = useState('');
  const [apiSecret, setApiSecret] = useState('');
  const [requestToken, setRequestToken] = useState('');
  const [step, setStep] = useState<'credentials' | 'token'>('credentials');
  const [loading, setLoading] = useState(false);
  const [loginURL, setLoginURL] = useState('');

  const handleGetLoginURL = async () => {
    if (!apiKey.trim()) {
      onError('API Key is required');
      return;
    }

    setLoading(true);
    try {
      const response = await fetch('/api/auth/login-url', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ apiKey }),
      });

      const data = await response.json();

      if (!response.ok) {
        throw new Error(data.error || 'Failed to get login URL');
      }

      setLoginURL(data.loginURL);
      setStep('token');
    } catch (error: any) {
      onError(error.message);
    } finally {
      setLoading(false);
    }
  };

  const handleGenerateToken = async () => {
    if (!apiKey.trim() || !apiSecret.trim() || !requestToken.trim()) {
      onError('All fields are required');
      return;
    }

    setLoading(true);
    try {
      const response = await fetch('/api/auth/generate-token', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          apiKey,
          apiSecret,
          requestToken,
        }),
      });

      const data = await response.json();

      if (!response.ok) {
        throw new Error(data.error || 'Authentication failed');
      }

      // Fetch user profile
      const profileResponse = await fetch(`/api/auth/profile/${data.sessionId}`);
      const profile = profileResponse.ok ? await profileResponse.json() : null;

      onLogin({
        sessionId: data.sessionId,
        accessToken: data.accessToken,
        profile,
      });
    } catch (error: any) {
      onError(error.message);
    } finally {
      setLoading(false);
    }
  };

  const handleBack = () => {
    setStep('credentials');
    setRequestToken('');
    setLoginURL('');
  };

  return (
    <div className="card">
      <h2 style={{ marginBottom: '24px', display: 'flex', alignItems: 'center', gap: '12px' }}>
        <Key size={24} />
        Kite Connect Authentication
      </h2>

      {step === 'credentials' ? (
        <div>
          <div className="form-group">
            <label className="form-label">API Key</label>
            <input
              type="text"
              className="form-input"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="Enter your Kite Connect API Key"
            />
          </div>

          <div className="form-group">
            <label className="form-label">API Secret</label>
            <input
              type="password"
              className="form-input"
              value={apiSecret}
              onChange={(e) => setApiSecret(e.target.value)}
              placeholder="Enter your Kite Connect API Secret"
            />
          </div>

          <button
            className="btn btn-primary"
            onClick={handleGetLoginURL}
            disabled={loading || !apiKey.trim()}
            style={{ width: '100%' }}
          >
            {loading ? 'Getting Login URL...' : 'Get Login URL'}
          </button>
        </div>
      ) : (
        <div>
          <div style={{ marginBottom: '20px', padding: '16px', background: '#f0f9ff', borderRadius: '8px', border: '1px solid #bae6fd' }}>
            <h3 style={{ marginBottom: '12px', color: '#0369a1' }}>Step 1: Login to Kite</h3>
            <p style={{ marginBottom: '12px', color: '#0c4a6e' }}>
              Click the button below to open Kite Connect login page in a new tab.
            </p>
            <a
              href={loginURL}
              target="_blank"
              rel="noopener noreferrer"
              className="btn btn-primary"
              style={{ display: 'inline-flex', alignItems: 'center', gap: '8px' }}
            >
              <ExternalLink size={16} />
              Open Kite Login
            </a>
          </div>

          <div className="form-group">
            <label className="form-label">Request Token</label>
            <input
              type="text"
              className="form-input"
              value={requestToken}
              onChange={(e) => setRequestToken(e.target.value)}
              placeholder="Paste the request token from the redirect URL"
            />
            <div className="success-message">
              After logging in, you'll be redirected to a URL containing the request token. Copy and paste it here.
            </div>
          </div>

          <div style={{ display: 'flex', gap: '12px' }}>
            <button
              className="btn btn-primary"
              onClick={handleGenerateToken}
              disabled={loading || !requestToken.trim()}
              style={{ flex: 1 }}
            >
              {loading ? 'Authenticating...' : 'Complete Authentication'}
            </button>
            <button
              className="btn"
              onClick={handleBack}
              disabled={loading}
              style={{ background: '#6b7280', color: 'white' }}
            >
              Back
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
