import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import './app/globals.css';
import BusinessApp from './components/business/app';
createRoot(document.getElementById('root')!).render(<BusinessApp />);
