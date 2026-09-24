"use client";

import dynamic from "next/dynamic";
import { Suspense } from "react";

const StellarProvider = dynamic(
  () => import("@/components/providers/stellar-provider").then((mod) => mod.StellarProvider),
  { ssr: false }
);

export function ClientProvider({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={null}>
      <StellarProvider>{children}</StellarProvider>
    </Suspense>
  );
}
