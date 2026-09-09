import type { Metadata } from 'next';
import '@fontsource-variable/inter';
import './globals.css';
export const metadata: Metadata = {
  title: 'Amux Business',
  description: 'Work, approvals, and automations in one place.',
};
export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
