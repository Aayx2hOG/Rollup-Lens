import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  ArrowRight,
  Blocks,
  CheckCircle2,
  Clock3,
  Copy,
  Database,
  Download,
  ExternalLink,
  Gauge,
  Layers3,
  PackageCheck,
  Play,
  RefreshCcw,
  ReceiptText,
  Search,
  Send,
  ShieldAlert,
  Upload,
  Wallet,
  XCircle,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import {
  API_BASE,
  api,
  type Account,
  type Block,
  type BlockSummary,
  type L1Deposit,
  type RollupBatch,
  type RollupBatchSummary,
  type SearchResult,
  type Transfer,
  type TransactionPoolEntry,
} from "./lib/api";
import { cn, formatNumber, pluralize, shortHash } from "./lib/utils";
import {
  connectWallet,
  depositToL2,
  hasWalletProvider,
  l1DepositInboxAddress,
  parseProofArtifact,
  settlementContractAddress,
  signTransfer,
  submitBatchWithProof,
} from "./lib/wallet";
import { Badge, IconButton, Panel, Shell, Skeleton } from "./components/ui";

const tabs = ["Overview", "Transactions", "Deposits", "Settlement", "Prove", "Operate", "Send"] as const;
type Tab = (typeof tabs)[number];

type TransactionReferenceFile = {
  format: "rolluplens.transaction-reference";
  version: 1;
  exported_at: string;
  api_base: string;
  transaction: {
    hash: string;
    block_height: number | null;
    transaction_index: number | null;
    from: string;
    to: string;
    amount: number;
    nonce: number;
  };
  batch: {
    number: number;
    status: string;
  } | null;
};

export function App() {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<Tab>("Overview");
  const [selectedHeight, setSelectedHeight] = useState<number | null>(null);
  const [selectedAccount, setSelectedAccount] = useState<string | null>(null);
  const [selectedTx, setSelectedTx] = useState<string | null>(null);
  const [selectedBatch, setSelectedBatch] = useState<number | null>(null);
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [connectionSeconds, setConnectionSeconds] = useState(0);
  const [overviewBlockTarget, setOverviewBlockTarget] = useState<number | null>(null);
  const [walletAddress, setWalletAddress] = useState<string | null>(null);
  const [walletError, setWalletError] = useState<string | null>(null);
  const [sendRecipient, setSendRecipient] = useState("");

  const readiness = useQuery({ queryKey: ["readiness"], queryFn: api.ready });
  const summary = useQuery({ queryKey: ["summary"], queryFn: api.summary });
  const blocks = useQuery({ queryKey: ["blocks"], queryFn: () => api.blocks() });
  const health = useQuery({ queryKey: ["health"], queryFn: api.health });
  const errors = useQuery({ queryKey: ["errors"], queryFn: api.errors });
  const rollupBatches = useQuery({ queryKey: ["rollup-batches"], queryFn: () => api.rollupBatches() });
  const l1Deposits = useQuery({ queryKey: ["l1-deposits"], queryFn: () => api.l1Deposits() });
  const transactionPool = useQuery({ queryKey: ["transaction-pool"], queryFn: () => api.transactionPool() });
  const block = useQuery({
    queryKey: ["block", selectedHeight],
    queryFn: () => api.block(selectedHeight!),
    enabled: selectedHeight !== null,
  });
  const account = useQuery({
    queryKey: ["account", selectedAccount],
    queryFn: () => api.account(selectedAccount!),
    enabled: Boolean(selectedAccount),
    retry: false,
  });
  const transaction = useQuery({
    queryKey: ["transaction", selectedTx],
    queryFn: () => api.transaction(selectedTx!),
    enabled: Boolean(selectedTx),
    retry: false,
  });
  const rollupBatch = useQuery({
    queryKey: ["rollup-batch", selectedBatch],
    queryFn: () => api.rollupBatch(selectedBatch!),
    enabled: selectedBatch !== null,
  });
  const connectionError = firstError(readiness.error, summary.error, blocks.error);
  const connectionMessage = connectionError ?? "Backend request failed";
  const hasCoreData = Boolean(summary.data && blocks.data);
  const databaseUnavailable = Boolean(
    readiness.error instanceof Error && readiness.error.message.toLowerCase().includes("database"),
  );
  const isWaking = Boolean(connectionError && !hasCoreData && !databaseUnavailable && connectionSeconds < 60);
  const isOffline = Boolean(connectionError && !hasCoreData && !databaseUnavailable && !isWaking);
  const isStale = Boolean(connectionError && hasCoreData);
  const readStatus = databaseUnavailable
    ? "database"
    : isWaking
      ? "waking"
    : isOffline
      ? "offline"
      : isStale
        ? "stale"
        : hasCoreData
          ? health.data?.status === "degraded" ? "degraded" : "ok"
          : "connecting";
  const blockItems = blocks.data?.items ?? [];
  const blockTotal = blocks.data?.total ?? blockItems.length;
  const batchItems = rollupBatches.data?.items ?? [];
  const batchTotal = rollupBatches.data?.total ?? batchItems.length;

  useEffect(() => {
    if (hasCoreData || databaseUnavailable) return;
    const timer = window.setInterval(() => setConnectionSeconds((seconds) => seconds + 1), 1000);
    return () => window.clearInterval(timer);
  }, [hasCoreData, databaseUnavailable]);

  useEffect(() => {
    if (selectedHeight === null && blockItems[0]) {
      setSelectedHeight(blockItems[0].height);
    }
  }, [blockItems, selectedHeight]);

  useEffect(() => {
    if (selectedBatch === null && batchItems[0]) {
      setSelectedBatch(batchItems[0].batch_number);
    }
  }, [batchItems, selectedBatch]);

  useEffect(() => {
    if (overviewBlockTarget === null || block.data?.height !== overviewBlockTarget) return;
    const transactions = block.data.transaction_views;
    if (transactions.length === 1) {
      setSelectedTx(transactions[0].hash);
      window.setTimeout(() => {
        document.getElementById("selected-transaction")?.scrollIntoView({
          behavior: "smooth",
          block: "start",
        });
      }, 100);
    }
    setOverviewBlockTarget(null);
  }, [block.data, overviewBlockTarget]);

  const accountsFromBlocks = useMemo(() => {
    const seen = new Map<string, { address: string; touches: number; sent: number; received: number }>();
    const selected = block.data;
    if (!selected) return [];
    for (const tx of selected.transactions) {
      const from = seen.get(tx.from) ?? { address: tx.from, touches: 0, sent: 0, received: 0 };
      from.touches += 1;
      from.sent += tx.amount;
      seen.set(tx.from, from);
      const to = seen.get(tx.to) ?? { address: tx.to, touches: 0, sent: 0, received: 0 };
      to.touches += 1;
      to.received += tx.amount;
      seen.set(tx.to, to);
    }
    return [...seen.values()];
  }, [block.data]);

  async function refreshAll() {
    await queryClient.invalidateQueries();
  }

  async function connectGlobalWallet() {
    setWalletError(null);
    try {
      setWalletAddress(await connectWallet());
    } catch (error) {
      setWalletError(error instanceof Error ? error.message : "Wallet connection failed");
    }
  }

  async function onSearch(query: string) {
    const trimmed = query.trim();
    if (!trimmed) return;
    setSearchError(null);
    try {
      const result = await api.search(trimmed);
      setSearchResult(result);
      if (result.kind === "Block") {
        setSelectedHeight(result.data.height);
        setActiveTab("Transactions");
      }
      if (result.kind === "Account") {
        setSelectedAccount(result.data.address);
        setActiveTab("Transactions");
      }
      if (result.kind === "Transaction") {
        setSelectedTx(result.data.hash);
        setActiveTab("Transactions");
      }
    } catch (error) {
      setSearchResult(null);
      setSearchError(error instanceof Error ? error.message : "Search failed");
    }
  }

  return (
    <Shell>
      <header className="sticky top-0 z-30 -mx-4 border-b border-white/10 bg-ink/90 px-4 py-3 backdrop-blur-xl sm:-mx-6 sm:px-6 lg:-mx-8 lg:px-8">
        <div className="mx-auto flex max-w-[82rem] flex-wrap items-center gap-x-5 gap-y-3">
          <button
            onClick={() => setActiveTab("Overview")}
            className="flex shrink-0 items-center gap-2.5 rounded-md text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-mint/50"
            aria-label="Go to overview"
          >
            <span className="flex h-8 w-8 items-center justify-center rounded-md bg-mint text-ink shadow-glow">
              <Layers3 className="h-4 w-4" />
            </span>
            <span className="text-base font-semibold tracking-tight text-white">RollupLens</span>
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                readStatus === "ok" ? "bg-mint" : "bg-amber",
              )}
              title={readStatusLabel(readStatus)}
            />
          </button>

          <HeaderNav activeTab={activeTab} onSelectTab={setActiveTab} />

          <div className="order-2 ml-auto flex min-w-0 flex-1 items-center justify-end gap-2 sm:flex-initial lg:order-none">
            {activeTab !== "Overview" && (
              <div className="min-w-0 flex-1 sm:flex-initial">
                <SearchBox onSearch={onSearch} pending={false} />
              </div>
            )}
            <WalletControl
              wallet={walletAddress}
              onConnect={connectGlobalWallet}
              onDisconnect={() => {
                setWalletAddress(null);
                setWalletError(null);
              }}
            />
            <IconButton onClick={refreshAll} title="Refresh data" aria-label="Refresh data" className="h-9 w-9">
              <RefreshCcw className="h-4 w-4" />
            </IconButton>
          </div>
        </div>
      </header>

      {walletError && (
        <div className="-mt-2 flex items-center justify-between gap-3 rounded-md border border-rose/20 bg-rose/10 px-3 py-2 text-sm text-rose">
          <span>{walletError}</span>
          <button onClick={() => setWalletError(null)} className="shrink-0 text-rose/70 hover:text-rose" aria-label="Dismiss wallet error">
            <XCircle className="h-4 w-4" />
          </button>
        </div>
      )}

      {!hasCoreData && (
        <ConnectionState
          status={readStatus}
          message={connectionMessage}
          onRetry={refreshAll}
        />
      )}
      {isStale && <StaleDataNotice message={connectionMessage} />}

      <AnimatePresence>
        {(searchResult || searchError) && (
          <motion.div
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
          >
            <SearchNotice result={searchResult} error={searchError} />
          </motion.div>
        )}
      </AnimatePresence>

      {activeTab === "Overview" && (
        <HomePage
          readStatus={readStatus}
          summary={summary.data}
          latestBatch={batchItems[0]}
          pendingTransfers={summary.data?.pending_transaction_count ?? 0}
          readyBatchCount={batchItems.filter((batch) => batch.status === "built").length}
          blocks={blockItems}
          blockTotal={blockTotal}
          onSearch={onSearch}
          onNavigate={setActiveTab}
          onSelectBlock={(height) => {
            setSelectedHeight(height);
            setOverviewBlockTarget(height);
            setActiveTab("Transactions");
          }}
        />
      )}

      {activeTab === "Transactions" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Blocks className="h-5 w-5" />}
            title="Transactions"
            description="Inspect transfers, their L2 inclusion, proof batch, and L1 settlement progress."
            meta={`${blockItems.length}/${blockTotal} blocks`}
          />
          <BlocksView
            blocks={blockItems}
            blockTotal={blockTotal}
            selectedHeight={selectedHeight}
            selectedBlock={block.data}
            blockLoading={block.isLoading}
            selectedTx={selectedTx}
            transaction={transaction.data}
            pool={transactionPool.data?.items ?? []}
            batches={batchItems}
            accountCandidates={accountsFromBlocks}
            selectedAccount={selectedAccount}
            account={account.data}
            accountError={account.error instanceof Error ? account.error.message : null}
            transactionError={transaction.error instanceof Error ? transaction.error.message : null}
            onSelectBlock={setSelectedHeight}
            onSelectAccount={(address) => {
              setSelectedAccount(address);
              setActiveTab("Transactions");
            }}
            onSelectTx={setSelectedTx}
            onSendTo={(address) => {
              setSendRecipient(address);
              setActiveTab("Send");
            }}
          />
        </div>
      )}

      {activeTab === "Settlement" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<PackageCheck className="h-5 w-5" />}
            title="Settlement"
            description="Verify which L2 batches reached Ethereum and inspect the state commitments and proof-related receipt data."
            meta={`${batchItems.length}/${batchTotal} batches`}
          />
          <RollupView
            batches={batchItems}
            batchTotal={batchTotal}
            selectedBatchNumber={selectedBatch}
            selectedBatch={rollupBatch.data}
            batchLoading={rollupBatch.isLoading}
            onSelectBatch={setSelectedBatch}
            onSelectTx={(hash) => {
              setSelectedTx(hash);
              setActiveTab("Transactions");
            }}
          />
        </div>
      )}

      {activeTab === "Prove" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<ShieldAlert className="h-5 w-5" />}
            title="Prove"
            description="Generate and submit validity proofs for settlement-ready batches."
            meta={pluralize(batchItems.filter((batch) => batch.status === "built").length, "ready batch")}
          />
          <ProofWorkspace
            batches={batchItems}
            selectedBatchNumber={selectedBatch}
            wallet={walletAddress}
            onSelectBatch={setSelectedBatch}
            onSubmitted={async (batchNumber) => {
              await queryClient.invalidateQueries({ queryKey: ["rollup-batches"] });
              await queryClient.invalidateQueries({ queryKey: ["rollup-batch", batchNumber] });
            }}
          />
        </div>
      )}

      {activeTab === "Deposits" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Upload className="h-5 w-5" />}
            title="Deposits"
            description="Track canonical L1 deposits observed for L2 recipients."
            meta={pluralize(l1Deposits.data?.total ?? 0, "deposit")}
          />
          <DepositsView
            deposits={l1Deposits.data?.items ?? []}
            batches={batchItems}
            total={l1Deposits.data?.total ?? 0}
            wallet={walletAddress}
            onWalletChange={setWalletAddress}
            onSelectBatch={(batchNumber) => {
              setSelectedBatch(batchNumber);
              setActiveTab("Settlement");
            }}
            onDeposited={async () => {
              await queryClient.invalidateQueries({ queryKey: ["l1-deposits"] });
            }}
          />
        </div>
      )}

      {activeTab === "Send" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Send className="h-5 w-5" />}
            title="Send funds"
            description="Connect an Ethereum wallet, choose a recipient, and sign an EIP-712 L2 transfer."
          />
          <SendTransferView
            wallet={walletAddress}
            recipient={sendRecipient}
            pool={transactionPool.data?.items ?? []}
            onWalletChange={setWalletAddress}
            onRecipientChange={setSendRecipient}
            onSubmitted={async (hash) => {
              setSelectedTx(hash);
              await refreshAll();
            }}
          />
        </div>
      )}

      {activeTab === "Operate" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Send className="h-5 w-5" />}
            title="Operate"
            description="Advanced controls for sequencing, queue inspection, and service health."
            meta={`${pluralize(summary.data?.pending_transaction_count ?? 0, "pending transfer")} / ${pluralize(confirmedDepositCount(l1Deposits.data?.items ?? []), "confirmed deposit")}`}
          />
          <SequencerView
            pool={transactionPool.data?.items ?? []}
            deposits={l1Deposits.data?.items ?? []}
            errors={errors.data ?? []}
            onRefresh={refreshAll}
            onOpenTransaction={(hash) => {
              setSelectedTx(hash);
              setActiveTab("Transactions");
            }}
            onSubmitted={async (hash) => {
              setSelectedTx(hash);
              await refreshAll();
            }}
            onSequenced={async (height) => {
              setSelectedHeight(height);
              await refreshAll();
            }}
          />
        </div>
      )}
    </Shell>
  );
}

function firstError(...errors: Array<unknown>) {
  for (const error of errors) {
    if (error instanceof Error) return error.message;
  }
  return null;
}

function readStatusLabel(status: string) {
  if (status === "ok") return "Live";
  if (status === "degraded") return "Degraded";
  if (status === "database") return "Database unavailable";
  if (status === "waking") return "Waking service";
  if (status === "stale") return "Last good data";
  if (status === "offline") return "Offline";
  return "Connecting";
}

function ConnectionState({
  status,
  message,
  onRetry,
}: {
  status: string;
  message: string;
  onRetry: () => Promise<void>;
}) {
  const connecting = status === "connecting";
  const waking = status === "waking";
  const database = status === "database";
  return (
    <Panel className={cn("p-4", database || status === "offline" ? "border-rose/25 bg-rose/10" : "border-amber/25 bg-amber/10")}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          {connecting || waking ? (
            <RefreshCcw className="mt-0.5 h-4 w-4 animate-spin text-amber" />
          ) : (
            <XCircle className="mt-0.5 h-4 w-4 text-rose" />
          )}
          <div>
            <div className="text-sm font-semibold text-white">
              {connecting
                ? "Connecting to the explorer API"
                : waking
                  ? "Waking the explorer service"
                  : database
                    ? "Database connection failed"
                    : "Explorer API unavailable"}
            </div>
            <div className="mt-1 text-sm text-white/58">
              {connecting
                ? "Starting the initial API request."
                : waking
                  ? "Free instances can take up to a minute to resume. Requests retry automatically."
                : database
                  ? "The API is online, but it cannot authenticate with Postgres. Verify DATABASE_URL and redeploy."
                  : message}
            </div>
          </div>
        </div>
        <button
          onClick={() => void onRetry()}
          className="inline-flex h-9 items-center justify-center gap-2 rounded-lg border border-white/12 px-3 text-sm text-white/72 hover:border-mint/30 hover:text-mint"
        >
          <RefreshCcw className="h-4 w-4" />
          Retry
        </button>
      </div>
    </Panel>
  );
}

function HeaderNav({ activeTab, onSelectTab }: { activeTab: Tab; onSelectTab: (tab: Tab) => void }) {
  const destinations = tabs.filter((tab) => tab !== "Send");

  return (
    <nav
      className="order-3 flex w-full min-w-0 items-center gap-1 overflow-x-auto border-t border-white/8 pt-2 lg:order-none lg:w-auto lg:border-0 lg:pt-0"
      aria-label="Primary navigation"
    >
      {destinations.map((tab) => (
        <button
          key={tab}
          onClick={() => onSelectTab(tab)}
          aria-current={activeTab === tab ? "page" : undefined}
          className={cn(
            "relative h-9 shrink-0 rounded-md px-2.5 text-sm text-white/50 transition-colors hover:bg-white/[0.05] hover:text-white",
            activeTab === tab && "text-white",
          )}
        >
          {activeTab === tab && (
            <motion.span
              layoutId="active-header-tab"
              className="absolute inset-x-2.5 bottom-0 h-px bg-mint"
              transition={{ type: "spring", bounce: 0.18, duration: 0.45 }}
            />
          )}
          <span className="relative">{tab}</span>
        </button>
      ))}
      <button
        onClick={() => onSelectTab("Send")}
        aria-current={activeTab === "Send" ? "page" : undefined}
        className={cn(
          "ml-1 inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md bg-mint px-3 text-sm font-semibold text-ink transition hover:bg-mint/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-mint/50",
          activeTab === "Send" && "ring-2 ring-white/25",
        )}
      >
        <Send className="h-3.5 w-3.5" />
        Send
      </button>
    </nav>
  );
}

function WalletControl({
  wallet,
  onConnect,
  onDisconnect,
}: {
  wallet: string | null;
  onConnect: () => Promise<void>;
  onDisconnect: () => void;
}) {
  if (wallet) {
    return (
      <button
        onClick={onDisconnect}
        title="Disconnect wallet"
        className="inline-flex h-9 shrink-0 items-center gap-2 rounded-md border border-mint/20 bg-mint/[0.08] px-3 text-sm font-medium text-mint transition hover:bg-mint/[0.14]"
      >
        <span className="h-1.5 w-1.5 rounded-full bg-mint" />
        <span className="hidden xl:inline">{shortHash(wallet, 6, 4)}</span>
        <span className="xl:hidden">Wallet</span>
      </button>
    );
  }

  return (
    <button
      onClick={() => void onConnect()}
      disabled={!hasWalletProvider()}
      title={hasWalletProvider() ? "Connect Ethereum wallet" : "No injected EVM wallet found"}
      className="inline-flex h-9 shrink-0 items-center gap-2 rounded-md border border-white/12 px-3 text-sm font-medium text-white/72 transition hover:border-mint/30 hover:text-mint disabled:cursor-not-allowed disabled:opacity-45"
    >
      <Wallet className="h-4 w-4" />
      <span>{hasWalletProvider() ? "Connect wallet" : "No wallet found"}</span>
    </button>
  );
}

function PageIntro({
  icon,
  title,
  description,
  meta,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  meta?: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-white/[0.025] p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-mint/20 bg-mint/10 text-mint">
            {icon}
          </div>
          <div>
            <h2 className="text-2xl font-semibold text-white">{title}</h2>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-white/58">{description}</p>
          </div>
        </div>
        {meta && <div className="shrink-0 text-sm text-white/48">{meta}</div>}
      </div>
    </section>
  );
}

function StaleDataNotice({ message }: { message: string }) {
  return (
    <Panel className="border-amber/25 bg-amber/10 p-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-semibold text-amber">
            <ShieldAlert className="h-4 w-4" />
            Showing last successful backend data
          </div>
          <div className="mt-2 text-sm text-white/68">{message}</div>
        </div>
        <div className="hash max-w-full rounded-md border border-white/10 bg-black/20 px-2 py-1 text-xs text-white/58 sm:max-w-md">
          {API_BASE}
        </div>
      </div>
    </Panel>
  );
}

function SearchBox({ onSearch, pending }: { onSearch: (query: string) => void; pending: boolean }) {
  const [value, setValue] = useState("");

  return (
    <form
      className="flex h-9 w-full min-w-0 items-center gap-2 rounded-md border border-white/10 bg-white/[0.045] px-2.5 transition focus-within:border-white/25 focus-within:bg-white/[0.07] sm:w-56 xl:w-64"
      onSubmit={(event) => {
        event.preventDefault();
        onSearch(value);
      }}
    >
      <Search className="h-4 w-4 shrink-0 text-white/42" />
      <input
        value={value}
        onChange={(event) => setValue(event.target.value)}
        placeholder="Search"
        className="min-w-0 flex-1 bg-transparent text-sm text-white outline-none placeholder:text-white/34"
      />
      <button
        type="submit"
        disabled={pending}
        className="sr-only"
      >
        Go
      </button>
    </form>
  );
}

function SearchNotice({ result, error }: { result: SearchResult | null; error: string | null }) {
  return (
    <Panel className="flex items-center justify-between gap-3 p-3">
      <div className="flex min-w-0 items-center gap-3">
        {error ? <XCircle className="h-5 w-5 shrink-0 text-rose" /> : <CheckCircle2 className="h-5 w-5 shrink-0 text-mint" />}
        <div className="min-w-0">
          <div className="text-sm font-medium text-white">{error ? "No match" : `${result?.kind} found`}</div>
          <div className="hash mt-0.5 max-w-[70vw] text-xs text-white/50">
            {error ?? summarizeResult(result)}
          </div>
        </div>
      </div>
    </Panel>
  );
}

function summarizeResult(result: SearchResult | null) {
  if (!result) return "";
  if (result.kind === "Block") return `height ${result.data.height} / ${result.data.block_hash}`;
  if (result.kind === "Account") return `${result.data.address} / balance ${result.data.balance}`;
  return `${result.data.hash} / block ${result.data.block_height ?? "pending"}`;
}

function HomePage({
  readStatus,
  summary,
  latestBatch,
  pendingTransfers,
  readyBatchCount,
  blocks,
  blockTotal,
  onSearch,
  onNavigate,
  onSelectBlock,
}: {
  readStatus: string;
  summary?: { height: number | null; transaction_count: number; account_count: number; valid: boolean };
  latestBatch?: RollupBatchSummary;
  pendingTransfers: number;
  readyBatchCount: number;
  blocks: BlockSummary[];
  blockTotal: number;
  onSearch: (query: string) => void;
  onNavigate: (tab: Tab) => void;
  onSelectBlock: (height: number) => void;
}) {
  const live = readStatus === "ok";

  return (
    <div className="grid gap-4">
      <section className="surface relative overflow-hidden rounded-lg px-5 py-8 sm:px-8 sm:py-10">
        <div className="!absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-mint/80 to-transparent" />
        <div className="pointer-events-none !absolute -right-32 -top-48 h-96 w-96 rounded-full bg-mint/[0.06] blur-3xl" />
        <div className="relative grid items-center gap-10 lg:grid-cols-[minmax(0,1.25fr)_minmax(20rem,0.75fr)]">
          <div>
            <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-[0.16em] text-white/48">
              <span className={cn("h-1.5 w-1.5 rounded-full", live ? "bg-mint shadow-[0_0_16px_rgba(88,244,183,0.9)]" : "bg-amber")} />
              {live ? "Network operational" : readStatusLabel(readStatus)}
            </div>
            <h1 className="mt-5 max-w-3xl text-3xl font-semibold leading-[1.12] tracking-[-0.035em] text-white sm:text-5xl">
              Verify the rollup.
              <span className="block text-white/42">From L2 activity to L1 finality.</span>
            </h1>
            <p className="mt-4 max-w-2xl text-sm leading-6 text-white/56 sm:text-base">
              Search blocks, transactions, and accounts. Track every batch from execution through proof submission and settlement.
            </p>
            <HomeSearch onSearch={onSearch} />
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <button
                onClick={() => onNavigate("Transactions")}
                className="inline-flex h-10 items-center gap-2 rounded-md bg-mint px-4 text-sm font-semibold text-ink transition hover:bg-mint/90"
              >
                Explore activity
                <ArrowRight className="h-4 w-4" />
              </button>
              <button
                onClick={() => onNavigate("Settlement")}
                className="inline-flex h-10 items-center gap-2 rounded-md border border-white/12 px-4 text-sm font-medium text-white/70 transition hover:border-white/25 hover:text-white"
              >
                View settlement
              </button>
              {readyBatchCount > 0 && (
                <button
                  onClick={() => onNavigate("Prove")}
                  className="inline-flex h-10 items-center gap-2 rounded-md px-3 text-sm font-medium text-amber transition hover:bg-amber/10"
                >
                  <ShieldAlert className="h-4 w-4" />
                  {pluralize(readyBatchCount, "batch")} ready to prove
                </button>
              )}
            </div>
          </div>
          <RollupFlow latestBatch={latestBatch} />
        </div>
      </section>

      <section className="grid grid-cols-2 overflow-hidden rounded-lg border border-white/10 bg-white/[0.025] lg:grid-cols-4">
        <HomeMetric label="Latest block" value={summary?.height ?? "—"} detail={`${formatNumber(blockTotal)} indexed`} />
        <HomeMetric label="Transactions" value={formatNumber(summary?.transaction_count)} detail={`${formatNumber(pendingTransfers)} pending`} />
        <HomeMetric label="Accounts" value={formatNumber(summary?.account_count)} detail={summary?.valid ? "State verified" : "State check required"} good={summary?.valid} />
        <HomeMetric
          label="Latest batch"
          value={latestBatch ? `#${latestBatch.batch_number}` : "—"}
          detail={latestBatch ? settlementStatusLabel(latestBatch.status) : "No batches yet"}
        />
      </section>

      <section className="grid gap-4 lg:grid-cols-[minmax(0,1.6fr)_minmax(20rem,0.8fr)]">
        <Panel className="overflow-hidden">
          <SectionTitle
            icon={<Activity className="h-4 w-4" />}
            title="Recent activity"
            action={
              <button onClick={() => onNavigate("Transactions")} className="inline-flex items-center gap-1 text-white/46 hover:text-mint">
                View all <ArrowRight className="h-3.5 w-3.5" />
              </button>
            }
          />
          {blocks.length === 0 ? (
            <div className="px-4 py-10 text-sm text-white/42">No blocks indexed yet.</div>
          ) : (
            <div className="divide-y divide-white/8">
              {blocks.slice(0, 6).map((block) => (
                <button
                  key={block.height}
                  onClick={() => onSelectBlock(block.height)}
                  className="grid w-full grid-cols-[4.5rem_minmax(0,1fr)_auto] items-center gap-3 px-4 py-3 text-left transition hover:bg-white/[0.04]"
                >
                  <span className="text-sm font-semibold text-white">#{block.height}</span>
                  <span className="hash text-xs text-white/46">{block.block_hash}</span>
                  <span className="flex items-center gap-3 text-xs text-white/42">
                    {pluralize(block.transaction_count, "tx")}
                    <ArrowRight className="h-3.5 w-3.5 text-white/24" />
                  </span>
                </button>
              ))}
            </div>
          )}
        </Panel>

        <Panel className="flex min-h-full flex-col p-5">
          <div className="flex items-center justify-between gap-3">
            <div className="text-sm font-semibold text-white">Settlement snapshot</div>
            {latestBatch && <Badge tone={settlementStatusTone(latestBatch.status)}>{settlementStatusLabel(latestBatch.status)}</Badge>}
          </div>
          {latestBatch ? (
            <>
              <div className="mt-6">
                <div className="text-xs uppercase tracking-[0.14em] text-white/36">Latest package</div>
                <div className="mt-2 text-3xl font-semibold tracking-tight text-white">#{latestBatch.batch_number}</div>
              </div>
              <div className="mt-5 grid grid-cols-2 gap-3">
                <HomeDetail label="Block range" value={`${latestBatch.start_block_height}–${latestBatch.end_block_height}`} />
                <HomeDetail label="Transfers" value={formatNumber(latestBatch.transaction_count)} />
                <HomeDetail label="Deposits" value={formatNumber(latestBatch.deposit_count)} />
                <HomeDetail label="L1 chain" value={latestBatch.l1_chain_id ?? "Pending"} />
              </div>
              <button
                onClick={() => onNavigate(latestBatch.status === "built" ? "Prove" : "Settlement")}
                className="mt-auto inline-flex h-10 items-center justify-between border-t border-white/10 pt-4 text-sm font-medium text-white/64 transition hover:text-mint"
              >
                {latestBatch.status === "built" ? "Open proof workspace" : "Inspect settlement"}
                <ArrowRight className="h-4 w-4" />
              </button>
            </>
          ) : (
            <div className="flex flex-1 items-center text-sm text-white/42">No settlement packages have been built.</div>
          )}
        </Panel>
      </section>
    </div>
  );
}

function RollupFlow({ latestBatch }: { latestBatch?: RollupBatchSummary }) {
  const reduceMotion = useReducedMotion();
  const stages = [
    {
      label: "L2 execution",
      detail: "Transactions sequenced",
      icon: <Blocks className="h-4 w-4" />,
      tone: "cyan",
    },
    {
      label: latestBatch ? `Batch #${latestBatch.batch_number}` : "Settlement batch",
      detail: "State commitments built",
      icon: <Layers3 className="h-4 w-4" />,
      tone: "mint",
    },
    {
      label: "Validity proof",
      detail: latestBatch?.status === "built" ? "Ready for prover" : "Proof verified",
      icon: <ShieldAlert className="h-4 w-4" />,
      tone: latestBatch?.status === "built" ? "amber" : "mint",
    },
    {
      label: "Ethereum L1",
      detail: latestBatch?.status === "finalized" ? "State finalized" : "Settlement destination",
      icon: <CheckCircle2 className="h-4 w-4" />,
      tone: latestBatch?.status === "finalized" ? "mint" : "neutral",
    },
  ] as const;

  return (
    <div className="relative hidden min-h-[22rem] items-center justify-center lg:flex" aria-label="Rollup settlement flow">
      <div className="!absolute inset-8 rounded-full border border-mint/[0.06] bg-mint/[0.025] blur-2xl" />
      <div className="relative w-full max-w-sm rounded-xl border border-white/10 bg-black/25 p-4 shadow-2xl shadow-black/25">
        <div className="mb-4 flex items-center justify-between border-b border-white/8 pb-3">
          <span className="text-xs font-medium uppercase tracking-[0.15em] text-white/40">Live settlement flow</span>
          <span className="flex items-center gap-1.5 text-xs text-mint/70">
            <motion.span
              className="h-1.5 w-1.5 rounded-full bg-mint"
              animate={reduceMotion ? undefined : { opacity: [0.35, 1, 0.35] }}
              transition={reduceMotion ? undefined : { duration: 2, repeat: Infinity }}
            />
            tracking
          </span>
        </div>
        <div>
          {stages.map((stage, index) => (
            <div key={stage.label}>
              <motion.div
                className="flex items-center gap-3 rounded-lg border border-white/8 bg-white/[0.035] p-3"
                animate={reduceMotion ? undefined : { borderColor: ["rgba(255,255,255,0.08)", "rgba(88,244,183,0.22)", "rgba(255,255,255,0.08)"] }}
                transition={reduceMotion ? undefined : { duration: 4, delay: index * 0.7, repeat: Infinity }}
              >
                <div className={cn(
                  "flex h-9 w-9 shrink-0 items-center justify-center rounded-md border",
                  stage.tone === "cyan" && "border-cyan/20 bg-cyan/10 text-cyan",
                  stage.tone === "mint" && "border-mint/20 bg-mint/10 text-mint",
                  stage.tone === "amber" && "border-amber/20 bg-amber/10 text-amber",
                  stage.tone === "neutral" && "border-white/10 bg-white/[0.04] text-white/40",
                )}>
                  {stage.icon}
                </div>
                <div className="min-w-0">
                  <div className="text-sm font-medium text-white/88">{stage.label}</div>
                  <div className="mt-0.5 text-xs text-white/40">{stage.detail}</div>
                </div>
                <motion.div
                  className="ml-auto h-1.5 w-1.5 rounded-full bg-mint"
                  animate={reduceMotion ? undefined : { scale: [0.8, 1.35, 0.8], opacity: [0.3, 1, 0.3] }}
                  transition={reduceMotion ? undefined : { duration: 2.4, delay: index * 0.45, repeat: Infinity }}
                />
              </motion.div>
              {index < stages.length - 1 && (
                <div className="relative ml-7 h-5 w-px overflow-hidden bg-white/10">
                  <motion.div
                    className="!absolute inset-x-0 h-2 bg-gradient-to-b from-mint/0 via-mint to-mint/0"
                    animate={reduceMotion ? undefined : { y: [-8, 20] }}
                    transition={reduceMotion ? undefined : { duration: 1.4, delay: index * 0.2, repeat: Infinity, ease: "linear" }}
                  />
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function HomeSearch({ onSearch }: { onSearch: (query: string) => void }) {
  const [value, setValue] = useState("");

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        onSearch(value);
      }}
      className="mt-7 flex h-12 w-full max-w-2xl items-center gap-3 rounded-lg border border-white/14 bg-black/25 px-4 shadow-lg shadow-black/10 transition focus-within:border-mint/45 focus-within:bg-black/35"
    >
      <Search className="h-5 w-5 shrink-0 text-white/36" />
      <input
        value={value}
        onChange={(event) => setValue(event.target.value)}
        placeholder="Search block, transaction, or address"
        aria-label="Search the rollup"
        className="min-w-0 flex-1 bg-transparent text-sm text-white outline-none placeholder:text-white/32"
      />
    </form>
  );
}

function HomeMetric({
  label,
  value,
  detail,
  good,
}: {
  label: string;
  value: React.ReactNode;
  detail: React.ReactNode;
  good?: boolean;
}) {
  return (
    <div className="min-w-0 border-b border-r border-white/8 p-4 last:border-r-0 lg:border-b-0">
      <div className="text-xs font-medium uppercase tracking-[0.13em] text-white/36">{label}</div>
      <div className="mt-3 truncate text-2xl font-semibold tracking-tight text-white">{value}</div>
      <div className={cn("mt-1.5 text-xs text-white/40", good && "text-mint/70")}>{detail}</div>
    </div>
  );
}

function HomeDetail({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="min-w-0 border-l border-white/10 pl-3">
      <div className="text-xs text-white/34">{label}</div>
      <div className="mt-1 truncate text-sm font-medium text-white/76">{value}</div>
    </div>
  );
}

function BlocksView({
  blocks,
  blockTotal,
  selectedHeight,
  selectedBlock,
  blockLoading,
  selectedTx,
  transaction,
  pool,
  batches,
  accountCandidates,
  selectedAccount,
  account,
  accountError,
  transactionError,
  onSelectBlock,
  onSelectAccount,
  onSelectTx,
  onSendTo,
}: {
  blocks: BlockSummary[];
  blockTotal: number;
  selectedHeight: number | null;
  selectedBlock?: Block;
  blockLoading: boolean;
  selectedTx: string | null;
  transaction?: { block_height: number | null; transaction_index: number; transfer: Transfer; hash: string };
  pool: TransactionPoolEntry[];
  batches: RollupBatchSummary[];
  accountCandidates: Array<{ address: string; touches: number; sent: number; received: number }>;
  selectedAccount: string | null;
  account?: Account;
  accountError: string | null;
  transactionError: string | null;
  onSelectBlock: (height: number) => void;
  onSelectAccount: (address: string) => void;
  onSelectTx: (hash: string) => void;
  onSendTo: (address: string) => void;
}) {
  const [lookupHash, setLookupHash] = useState("");
  const [referenceError, setReferenceError] = useState<string | null>(null);
  const poolEntry = selectedTx ? pool.find((entry) => entry.hash === selectedTx) : undefined;
  const visibleTransfer = transaction?.transfer ?? poolEntry?.transfer;
  const blockHeight = transaction?.block_height ?? poolEntry?.included_block_height ?? null;
  const batch = blockHeight === null
    ? undefined
    : batches.find((entry) => entry.start_block_height <= blockHeight && entry.end_block_height >= blockHeight);

  async function openReferenceFile(file: File) {
    setReferenceError(null);
    try {
      const parsed = JSON.parse(await file.text()) as Partial<TransactionReferenceFile>;
      if (
        parsed.format !== "rolluplens.transaction-reference"
        || parsed.version !== 1
        || typeof parsed.transaction?.hash !== "string"
        || !parsed.transaction.hash.trim()
      ) {
        throw new Error("This is not a valid RollupLens transaction reference file.");
      }
      setLookupHash(parsed.transaction.hash);
      onSelectTx(parsed.transaction.hash);
    } catch (error) {
      setReferenceError(error instanceof Error ? error.message : "Could not read transaction reference file.");
    }
  }

  return (
    <div className="grid grid-cols-1 items-start gap-4 xl:grid-cols-[23rem_1fr]">
      <div className="grid content-start gap-4">
        <Panel className="overflow-hidden">
          <SectionTitle icon={<Blocks className="h-4 w-4" />} title="Activity groups" action={`${blocks.length}/${blockTotal} indexed`} />
          <div className="max-h-[36rem] overflow-auto">
            {blocks.map((block) => (
              <button
                key={block.height}
                onClick={() => onSelectBlock(block.height)}
                className={cn(
                  "flex w-full items-center gap-3 border-b border-white/8 px-4 py-3 text-left transition",
                  selectedHeight === block.height ? "bg-mint/10" : "hover:bg-white/[0.045]",
                )}
              >
                <div className="flex h-9 w-12 shrink-0 items-center justify-center rounded-md bg-white/[0.06] text-sm font-semibold text-white">
                  {block.height}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="hash text-sm text-white/82">{block.block_hash}</div>
                  <div className="mt-1 text-xs text-white/42">{pluralize(block.transaction_count, "transfer")}</div>
                </div>
              </button>
            ))}
          </div>
        </Panel>
        <AccountFinder
          candidates={accountCandidates}
          selectedAccount={selectedAccount}
          onSelectAccount={onSelectAccount}
        />
      </div>

      <div className="grid gap-4">
        <Panel className="p-4">
          <SectionTitle icon={<Database className="h-4 w-4" />} title="Activity group detail" action={selectedBlock ? `block ${selectedBlock.height}` : "none"} flush />
          {blockLoading ? (
            <Skeleton className="mt-4 h-56" />
          ) : selectedBlock ? (
            <div className="mt-4 grid gap-3">
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                <HashLine label="Block hash" value={selectedBlock.block_hash} />
                <HashLine label="Parent block hash" value={selectedBlock.parent_hash} />
                <HashLine label="State root" value={selectedBlock.state_root} />
                <MetricLine label="Transfers" value={selectedBlock.transactions.length} />
              </div>
              <div className="overflow-hidden rounded-lg border border-white/10">
                <div className="grid grid-cols-[minmax(0,1fr)_5rem_minmax(0,1fr)_4rem] gap-3 border-b border-white/10 bg-white/[0.045] px-3 py-2 text-center text-xs uppercase tracking-[0.14em] text-white/38">
                  <span>Sender</span>
                  <span className="text-center">Amount</span>
                  <span>Receiver</span>
                  <span>Details</span>
                </div>
                {selectedBlock.transactions.length === 0 ? (
                  <div className="px-3 py-6 text-sm text-white/48">The first block only starts the system, so it has no transfers.</div>
                ) : (
                  selectedBlock.transactions.map((tx, index) => (
                    <TransactionRow
                      key={`${tx.signature}-${index}`}
                      tx={tx}
                      hash={selectedBlock.transaction_views[index]?.hash ?? null}
                      onSelectAccount={onSelectAccount}
                      onSelectTx={onSelectTx}
                    />
                  ))
                )}
              </div>
            </div>
          ) : (
            <div className="mt-4 text-sm text-white/50">Select an activity group from the list.</div>
          )}
        </Panel>

        <Panel id="selected-transaction" className="scroll-mt-40 p-4">
          <SectionTitle icon={<Activity className="h-4 w-4" />} title="Selected transaction" action={selectedTx ? shortHash(selectedTx) : "none selected"} flush />
          <form
            className="mt-4 flex flex-wrap gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              const hash = lookupHash.trim();
              if (hash) onSelectTx(hash);
            }}
          >
            <input
              value={lookupHash}
              onChange={(event) => setLookupHash(event.target.value)}
              placeholder="Paste transaction hash"
              aria-label="Transaction hash"
              className="h-10 min-w-[16rem] flex-1 rounded-lg border border-white/10 bg-white/[0.06] px-3 font-mono text-sm text-white outline-none placeholder:font-sans placeholder:text-white/34 focus:border-mint/40"
            />
            <button
              type="submit"
              disabled={!lookupHash.trim()}
              className="inline-flex h-10 items-center gap-2 rounded-lg bg-mint px-3 text-sm font-semibold text-ink transition hover:bg-mint/90 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <Search className="h-4 w-4" />
              Open
            </button>
            <label className="inline-flex h-10 cursor-pointer items-center gap-2 rounded-lg border border-white/12 px-3 text-sm font-medium text-white/68 transition hover:border-mint/30 hover:text-mint">
              <Upload className="h-4 w-4" />
              Open reference
              <input
                type="file"
                accept="application/json,.json"
                className="hidden"
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (file) void openReferenceFile(file);
                  event.target.value = "";
                }}
              />
            </label>
          </form>
          {referenceError && <div className="mt-3 text-sm text-rose">{referenceError}</div>}
          {selectedTx && visibleTransfer ? (
            <div className="mt-4 grid gap-4">
              <div className="flex flex-col gap-3 rounded-lg border border-white/10 bg-white/[0.035] p-3 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <div className="text-sm font-semibold text-white">Portable transaction reference</div>
                  <div className="mt-1 text-xs text-white/46">
                    The transaction hash is the stable lookup ID. Export it with public metadata to reopen this transaction later.
                  </div>
                </div>
                <button
                  onClick={() => downloadTransactionReference({
                    hash: selectedTx,
                    transfer: visibleTransfer,
                    blockHeight,
                    transactionIndex: transaction?.transaction_index ?? poolEntry?.included_transaction_index ?? null,
                    batch,
                  })}
                  className="inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-lg border border-mint/25 bg-mint/10 px-3 text-sm font-medium text-mint hover:bg-mint/15"
                >
                  <Download className="h-4 w-4" />
                  Export reference
                </button>
              </div>
              <TransactionLifecycle
                hash={selectedTx}
                poolEntry={poolEntry}
                blockHeight={blockHeight}
                batch={batch}
              />
              <div className="grid gap-3 md:grid-cols-2">
                <HashLine label="Transaction hash" value={selectedTx} />
                <MetricLine label="Activity group" value={blockHeight ?? "waiting"} />
                <TransferDetail tx={visibleTransfer} onSelectAccount={onSelectAccount} />
              </div>
            </div>
          ) : transactionError ? (
            <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{transactionError}</div>
          ) : (
            <div className="mt-4 text-sm text-white/50">
              Paste a transaction hash above or select the details icon in an activity row.
            </div>
          )}
        </Panel>
        <AccountDetail
          selectedAccount={selectedAccount}
          account={account}
          accountError={accountError}
          onSendTo={onSendTo}
        />
      </div>
    </div>
  );
}

function TransactionLifecycle({
  hash,
  poolEntry,
  blockHeight,
  batch,
}: {
  hash: string;
  poolEntry?: TransactionPoolEntry;
  blockHeight: number | null;
  batch?: RollupBatchSummary;
}) {
  const failed = poolEntry?.status === "failed" || poolEntry?.status === "expired";
  const steps = [
    {
      label: "Submitted",
      complete: Boolean(poolEntry || blockHeight !== null),
      detail: failed ? poolEntry?.failure_reason ?? poolEntry?.status : poolEntry?.created_at ?? shortHash(hash),
    },
    {
      label: "Sequenced",
      complete: blockHeight !== null,
      detail: blockHeight === null ? "Waiting for an L2 block" : `Included in block ${blockHeight}`,
    },
    {
      label: "Proven",
      complete: Boolean(batch && batch.status !== "built"),
      detail: !batch
        ? "Waiting for a validity batch"
        : batch.status === "built"
          ? `Batch ${batch.batch_number} is ready for proof`
          : `Batch ${batch.batch_number} proof submitted`,
    },
    {
      label: "Settled",
      complete: batch?.status === "finalized",
      detail: batch?.status === "finalized"
        ? `Finalized on chain ${batch.l1_chain_id ?? "L1"}`
        : "Waiting for L1 finalization",
    },
  ];

  return (
    <div className="rounded-lg border border-white/10 bg-black/15 p-3">
      <div className="mb-3 text-xs font-medium uppercase tracking-[0.16em] text-white/38">Lifecycle</div>
      <div className="grid gap-2 md:grid-cols-4">
        {steps.map((step, index) => (
          <div key={step.label} className="relative min-w-0 rounded-lg border border-white/8 bg-white/[0.035] p-3">
            <div className="flex items-center gap-2">
              <div className={cn(
                "flex h-6 w-6 items-center justify-center rounded-full border text-xs font-semibold",
                failed && index === 0
                  ? "border-rose/30 bg-rose/10 text-rose"
                  : step.complete
                    ? "border-mint/30 bg-mint/10 text-mint"
                    : "border-white/10 bg-white/[0.04] text-white/38",
              )}>
                {step.complete ? <CheckCircle2 className="h-3.5 w-3.5" /> : index + 1}
              </div>
              <span className="text-sm font-semibold text-white">{step.label}</span>
            </div>
            <div className="mt-2 truncate text-xs text-white/44" title={step.detail}>{step.detail}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function AccountFinder({
  candidates,
  selectedAccount,
  onSelectAccount,
}: {
  candidates: Array<{ address: string; touches: number; sent: number; received: number }>;
  selectedAccount: string | null;
  onSelectAccount: (address: string) => void;
}) {
  const [manual, setManual] = useState("");

  return (
    <Panel className="p-4">
      <SectionTitle icon={<Wallet className="h-4 w-4" />} title="Find a wallet" action="Current balance" flush />
      <form
        className="mt-4 flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (manual.trim()) onSelectAccount(manual.trim());
        }}
      >
        <input
          value={manual}
          onChange={(event) => setManual(event.target.value)}
          placeholder="Paste wallet address"
          className="min-w-0 flex-1 rounded-lg border border-white/10 bg-white/[0.06] px-3 py-2 text-sm text-white outline-none placeholder:text-white/34 focus:border-mint/40"
        />
        <button className="rounded-lg bg-mint px-3 py-2 text-sm font-semibold text-ink transition hover:bg-mint/90">
          Open
        </button>
      </form>
      <div className="mt-4 space-y-2">
        {candidates.map((candidate) => (
          <button
            key={candidate.address}
            onClick={() => onSelectAccount(candidate.address)}
            className={cn(
              "w-full rounded-lg border p-3 text-left transition",
              selectedAccount === candidate.address
                ? "border-mint/35 bg-mint/10"
                : "border-white/10 bg-white/[0.035] hover:border-white/20",
            )}
          >
            <div className="hash text-sm text-white">{candidate.address}</div>
            <div className="mt-2 flex gap-2 text-xs text-white/48">
              <span>{pluralize(candidate.touches, "transfer")}</span>
              <span>sent {candidate.sent}</span>
              <span>received {candidate.received}</span>
            </div>
          </button>
        ))}
      </div>
    </Panel>
  );
}

function AccountDetail({
  selectedAccount,
  account,
  accountError,
  onSendTo,
}: {
  selectedAccount: string | null;
  account?: Account;
  accountError: string | null;
  onSendTo: (address: string) => void;
}) {
  return (
    <Panel className="p-4">
      <SectionTitle icon={<Wallet className="h-4 w-4" />} title="Wallet detail" action={selectedAccount ? shortHash(selectedAccount) : "none"} flush />
      {account ? (
        <div className="mt-4 grid gap-3">
          <div className="grid gap-3 md:grid-cols-3">
            <HashLine label="Wallet address" value={account.address} />
            <MetricLine label="Balance" value={account.balance} />
            <MetricLine label="Next transfer #" value={account.nonce} />
          </div>
          <div>
            <button
              onClick={() => onSendTo(account.address)}
              className="inline-flex h-10 items-center gap-2 rounded-lg bg-mint px-4 text-sm font-semibold text-ink transition hover:bg-mint/90"
            >
              <Send className="h-4 w-4" />
              Send to this wallet
            </button>
          </div>
        </div>
      ) : accountError ? (
        <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{accountError}</div>
      ) : (
        <div className="mt-4 text-sm text-white/50">Select a wallet from the current activity group or paste an address.</div>
      )}
    </Panel>
  );
}

function SendTransferView({
  wallet,
  recipient,
  pool,
  onWalletChange,
  onRecipientChange,
  onSubmitted,
}: {
  wallet: string | null;
  recipient: string;
  pool: TransactionPoolEntry[];
  onWalletChange: (wallet: string | null) => void;
  onRecipientChange: (recipient: string) => void;
  onSubmitted: (hash: string) => Promise<void>;
}) {
  const [amount, setAmount] = useState(1);
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<{ tone: "good" | "bad" | "neutral"; message: string } | null>(null);
  const walletAddress = wallet;
  const walletAccount = useQuery({
    queryKey: ["account", walletAddress],
    queryFn: () => api.account(walletAddress!),
    enabled: Boolean(walletAddress),
    retry: false,
  });
  const pendingFromWallet = walletAddress
    ? pool.filter((entry) => entry.status === "pending" && entry.transfer.from === walletAddress).length
    : 0;
  const nextNonce = (walletAccount.data?.nonce ?? 0) + pendingFromWallet;

  async function sendTransfer() {
    if (!wallet || !walletAccount.data) return;
    setSubmitting(true);
    setNotice(null);
    try {
      const transfer = await signTransfer(wallet, recipient.trim(), amount, nextNonce);
      const submitted = await api.submitTransaction(transfer);
      onRecipientChange("");
      setNotice({
        tone: "good",
        message: `Transfer ${shortHash(submitted.hash, 10, 8)} submitted successfully.`,
      });
      await onSubmitted(submitted.hash);
    } catch (error) {
      setNotice({ tone: "bad", message: error instanceof Error ? error.message : "Transfer failed" });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Panel className="mx-auto w-full max-w-3xl overflow-hidden">
      <div className="border-b border-white/8 bg-white/[0.025] p-5">
        <div className="flex items-center justify-between gap-3">
          <div>
            <div className="text-lg font-semibold text-white">Send L2 funds</div>
            <div className="mt-1 text-sm text-white/48">Your private key stays inside your wallet.</div>
          </div>
          {wallet && <Badge tone="good">Ethereum wallet connected</Badge>}
        </div>
      </div>

      <div className="grid gap-5 p-5">
        {!wallet ? (
          <div className="rounded-lg border border-white/10 bg-white/[0.035] p-4 text-sm text-white/58">
            Connect your Ethereum wallet from the header to send L2 funds.
          </div>
        ) : (
          <>
            <div className="grid gap-3 rounded-lg border border-white/10 bg-white/[0.035] p-4 sm:grid-cols-[1fr_auto] sm:items-center">
              <div className="min-w-0">
                <div className="text-xs uppercase tracking-[0.14em] text-white/38">From</div>
                <div className="hash mt-1 text-sm text-white/78">{walletAddress}</div>
              </div>
              <div className="flex items-center gap-5">
                <div>
                  <div className="text-xs text-white/38">Balance</div>
                  <div className="mt-1 text-lg font-semibold text-white">{walletAccount.data?.balance ?? "—"}</div>
                </div>
                <button
                  onClick={() => onWalletChange(null)}
                  className="text-sm text-white/46 hover:text-white"
                >
                  Disconnect
                </button>
              </div>
            </div>

            {walletAccount.error ? (
              <div className="rounded-lg border border-amber/25 bg-amber/10 p-3 text-sm text-amber">
                This wallet is not funded on the current rollup.
              </div>
            ) : (
              <div className="grid gap-4">
                <label className="grid gap-2 text-sm font-medium text-white/70">
                  Recipient
                  <input
                    value={recipient}
                    onChange={(event) => onRecipientChange(event.target.value)}
                    placeholder="Paste recipient wallet address"
                    className="h-12 rounded-lg border border-white/10 bg-white/[0.06] px-3 font-mono text-sm text-white outline-none placeholder:text-white/30 focus:border-mint/50 focus:ring-2 focus:ring-mint/10"
                  />
                </label>
                <label className="grid gap-2 text-sm font-medium text-white/70">
                  Amount
                  <input
                    type="number"
                    min={1}
                    step={1}
                    value={amount}
                    onChange={(event) => setAmount(Number(event.target.value))}
                    className="h-12 rounded-lg border border-white/10 bg-white/[0.06] px-3 text-lg text-white outline-none focus:border-mint/50 focus:ring-2 focus:ring-mint/10"
                  />
                </label>
                <button
                  onClick={sendTransfer}
                  disabled={submitting || recipient.trim().length === 0 || amount < 1}
                  className="inline-flex h-12 items-center justify-center gap-2 rounded-lg bg-mint px-5 text-base font-semibold text-ink shadow-glow transition hover:bg-mint/90 disabled:cursor-not-allowed disabled:opacity-45"
                >
                  <Send className="h-5 w-5" />
                  {submitting ? "Sending…" : "Send transfer"}
                </button>
                <div className="text-center text-xs text-white/38">
                  The next nonce ({nextNonce}) is selected automatically.
                </div>
              </div>
            )}
          </>
        )}

        {notice && (
          <div className={cn(
            "rounded-lg border p-3 text-sm",
            notice.tone === "bad"
              ? "border-rose/25 bg-rose/10 text-rose"
              : notice.tone === "good"
                ? "border-mint/25 bg-mint/10 text-mint"
                : "border-white/10 bg-white/[0.04] text-white/64",
          )}>
            {notice.message}
          </div>
        )}
      </div>
    </Panel>
  );
}

function SequencerView({
  pool,
  deposits,
  errors,
  onRefresh,
  onOpenTransaction,
  onSubmitted,
  onSequenced,
}: {
  pool: TransactionPoolEntry[];
  deposits: L1Deposit[];
  errors: Array<{ id: number; severity: string; message: string; service: string; created_at: string; resolved_at: string | null }>;
  onRefresh: () => Promise<void>;
  onOpenTransaction: (hash: string) => void;
  onSubmitted: (hash: string) => Promise<void>;
  onSequenced: (height: number) => Promise<void>;
}) {
  const [transferJson, setTransferJson] = useState("");
  const [showSignedJson, setShowSignedJson] = useState(false);
  const [limit, setLimit] = useState(100);
  const [submitting, setSubmitting] = useState(false);
  const [sequencing, setSequencing] = useState(false);
  const [notice, setNotice] = useState<{ tone: "good" | "bad" | "neutral"; message: string } | null>(null);
  const pendingTotal = pool.filter((entry) => entry.status === "pending").length;
  const confirmedDeposits = deposits.filter((deposit) => deposit.status === "confirmed").length;
  const sequenceableTotal = pendingTotal + confirmedDeposits;
  const pendingEntries = pool.filter((entry) => entry.status === "pending");
  const recentIncluded = pool.filter((entry) => entry.status === "included").slice(0, 5);
  async function submitTransfer() {
    setSubmitting(true);
    setNotice(null);
    try {
      const parsed = JSON.parse(transferJson) as Transfer;
      const submitted = await api.submitTransaction(parsed);
      setTransferJson("");
      setNotice({ tone: "good", message: `Transfer ${shortHash(submitted.hash, 10, 8)} is waiting to be grouped.` });
      await onSubmitted(submitted.hash);
    } catch (error) {
      setNotice({ tone: "bad", message: error instanceof Error ? error.message : "Transaction submission failed" });
    } finally {
      setSubmitting(false);
    }
  }

  async function sequenceBatch() {
    setSequencing(true);
    setNotice(null);
    try {
      const result = await api.sequenceBatch(limit);
      setNotice({
        tone: "good",
        message: `Built activity group ${result.block.height} with ${pluralize(result.included_transactions.length, "transfer")} and ${pluralize(result.included_deposits.length, "deposit")}.`,
      });
      await onSequenced(result.block.height);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Batch sequencing failed";
      if (message.toLowerCase().includes("no pending transactions")) {
        setNotice({
          tone: "neutral",
          message: "The queue was already processed. Latest block data has been refreshed.",
        });
        await onRefresh();
      } else {
        setNotice({ tone: "bad", message });
      }
    } finally {
      setSequencing(false);
    }
  }

  return (
    <div className="grid grid-cols-1 items-start gap-4 xl:grid-cols-[0.85fr_1.15fr]">
      <div className="grid gap-4">
        <Panel className="p-4">
          <SectionTitle icon={<Upload className="h-4 w-4" />} title="Advanced transaction submission" action="operators" flush />
          <p className="mt-2 text-sm leading-6 text-white/48">
            Submit an externally signed transfer payload. Most users should use Send instead.
          </p>
          <button onClick={() => setShowSignedJson((shown) => !shown)} className="mt-3 text-sm text-mint/80 hover:text-mint">
            {showSignedJson ? "Hide" : "Show"} advanced signed JSON submission
          </button>
          {showSignedJson && (
            <div className="mt-3">
              <textarea
                value={transferJson}
                onChange={(event) => setTransferJson(event.target.value)}
                placeholder='{"from":"0x...","to":"0x...","amount":5,"nonce":0,"signature":"0x..."}'
                className="min-h-36 w-full resize-y rounded-lg border border-white/10 bg-black/20 p-3 font-mono text-sm leading-6 text-white outline-none placeholder:text-white/28 focus:border-mint/40"
              />
              <button onClick={submitTransfer} disabled={submitting || transferJson.trim().length === 0} className="mt-3 rounded-lg border border-mint/30 px-3 py-2 text-sm font-medium text-mint disabled:opacity-50">
                Submit signed JSON
              </button>
            </div>
          )}
        </Panel>

        <Panel className="p-4">
          <SectionTitle
            icon={<Play className="h-4 w-4" />}
            title="Build the next L2 block"
            action={`${pluralize(pendingTotal, "pending transfer")} / ${pluralize(confirmedDeposits, "confirmed deposit")}`}
            flush
          />
          <p className="mt-2 text-sm leading-6 text-white/48">
            Sequence pending transfers and confirmed L1 deposits into the next block.
          </p>
          <div className="mt-4 grid gap-3 sm:grid-cols-[1fr_auto]">
            <label className="grid gap-2 text-sm text-white/62">
              Transfer limit
              <input
                type="number"
                min={1}
                max={100}
                value={limit}
                onChange={(event) => setLimit(Math.min(100, Math.max(1, Number(event.target.value) || 1)))}
                className="h-10 rounded-lg border border-white/10 bg-white/[0.06] px-3 text-white outline-none focus:border-mint/40"
              />
            </label>
            <button
              onClick={sequenceBatch}
              disabled={sequencing || sequenceableTotal === 0}
              className="self-end inline-flex h-10 items-center justify-center gap-2 rounded-lg bg-cyan px-3 text-sm font-semibold text-ink transition hover:bg-cyan/90 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/30"
            >
              <Play className="h-4 w-4" />
              Build block
            </button>
          </div>
          {pendingTotal === 0 && (
            <div className="mt-3 text-xs text-white/40">
              {confirmedDeposits > 0 ? "Confirmed deposits are ready to sequence." : "Waiting for a submitted transfer or confirmed deposit."}
            </div>
          )}
        </Panel>

        {notice && (
          <Panel className={cn("p-3", notice.tone === "bad" ? "border-rose/25 bg-rose/10" : "border-mint/25 bg-mint/10")}>
            <div className={cn("text-sm font-medium", notice.tone === "bad" ? "text-rose" : "text-mint")}>{notice.message}</div>
          </Panel>
        )}
      </div>

      <div className="grid content-start gap-4">
        <Panel className="overflow-hidden">
          <SectionTitle
            icon={<Activity className="h-4 w-4" />}
            title="Pending transfers"
            action={pluralize(pendingEntries.length, "waiting")}
          />
          {pendingEntries.length === 0 ? (
            <div className="flex items-center justify-between gap-3 px-4 py-5">
              <span className="text-sm text-white/48">The queue is empty.</span>
              <Badge tone="good">All processed</Badge>
            </div>
          ) : (
            <div className="max-h-[24rem] overflow-auto">
              {pendingEntries.map((entry) => (
                <TransferQueueRow key={entry.id} entry={entry} />
              ))}
            </div>
          )}
        </Panel>

        <Panel className="overflow-hidden">
          <SectionTitle
            icon={<Blocks className="h-4 w-4" />}
            title="Recent sequencing activity"
            action={`${recentIncluded.length} recent`}
          />
          {recentIncluded.length === 0 ? (
            <div className="px-4 py-5 text-sm text-white/48">No transfers have been included yet.</div>
          ) : (
            <div className="divide-y divide-white/8">
              {recentIncluded.map((entry) => (
                <div key={entry.id} className="flex items-center gap-3 px-4 py-3">
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-mint/10 text-sm font-semibold text-mint">
                    {entry.included_block_height ?? "–"}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="hash text-sm text-white/78">{entry.hash}</div>
                    <div className="mt-1 text-xs text-white/44">
                      {entry.transfer.amount} transferred · position {entry.included_transaction_index ?? "–"}
                    </div>
                  </div>
                  <button
                    onClick={() => onOpenTransaction(entry.hash)}
                    title="Open transaction details"
                    aria-label="Open transaction details"
                    className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-white/10 text-white/48 hover:border-mint/30 hover:text-mint"
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </Panel>
        <SystemIssuesPanel errors={errors} />
      </div>
    </div>
  );
}

function TransferQueueRow({ entry }: { entry: TransactionPoolEntry }) {
  return (
    <div className="grid gap-3 border-b border-white/8 px-4 py-3 last:border-b-0">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="hash min-w-0 text-sm text-white">{entry.hash}</div>
        <Badge tone="warn">pending</Badge>
      </div>
      <div className="grid gap-2 text-xs text-white/48 sm:grid-cols-3">
        <span className="hash">sender {shortHash(entry.transfer.from, 8, 6)}</span>
        <span className="hash">receiver {shortHash(entry.transfer.to, 8, 6)}</span>
        <span>amount {entry.transfer.amount} / transfer #{entry.transfer.nonce}</span>
      </div>
    </div>
  );
}

function SystemIssuesPanel({
  errors,
}: {
  errors: Array<{ id: number; severity: string; message: string; service: string; created_at: string; resolved_at: string | null }>;
}) {
  return (
    <Panel className="overflow-hidden">
      <SectionTitle icon={<ShieldAlert className="h-4 w-4" />} title="System issues" action={`${errors.length} recent`} />
      {errors.length === 0 ? (
        <div className="flex items-center justify-between gap-3 px-4 py-5">
          <span className="text-sm text-white/48">No active API or sequencing issues.</span>
          <Badge tone="good">Healthy</Badge>
        </div>
      ) : (
        <div className="divide-y divide-white/8">
          {errors.map((error) => (
            <div key={error.id} className="grid gap-1 px-4 py-3">
              <div className="flex items-center gap-2">
                <Badge tone={error.resolved_at ? "neutral" : error.severity === "warning" ? "warn" : "bad"}>{error.severity}</Badge>
                <span className="text-sm font-medium text-white">{error.message}</span>
              </div>
              <div className="text-xs text-white/44">{error.service} / {error.created_at}</div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function DepositsView({
  deposits,
  batches,
  total,
  wallet,
  onWalletChange,
  onSelectBatch,
  onDeposited,
}: {
  deposits: L1Deposit[];
  batches: RollupBatchSummary[];
  total: number;
  wallet: string | null;
  onWalletChange: (wallet: string | null) => void;
  onSelectBatch: (batchNumber: number) => void;
  onDeposited: () => Promise<void>;
}) {
  const observed = deposits.filter((deposit) => deposit.status === "observed").length;
  const confirmed = deposits.filter((deposit) => deposit.status === "confirmed").length;
  const executed = deposits.filter((deposit) => deposit.status === "executed" || deposit.status === "finalized").length;
  const finalized = deposits.filter((deposit) => deposit.status === "finalized").length;
  const orphaned = deposits.filter((deposit) => deposit.status === "orphaned").length;
  const latestDepositBatch = batches.find((batch) => batch.deposit_count > 0);

  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(29rem,32rem)_minmax(0,1fr)]">
      <div className="grid min-w-0 content-start gap-4">
        <DepositActionPanel
          wallet={wallet}
          onWalletChange={onWalletChange}
          onDeposited={onDeposited}
        />
        <Panel className="overflow-hidden">
          <SectionTitle icon={<Activity className="h-4 w-4" />} title="Deposit pipeline" action={`${deposits.length}/${total} indexed`} />
          <div className="grid gap-3 p-4">
            <DepositStageRow label="Observed" value={observed} complete={deposits.length > 0} />
            <DepositStageRow label="Confirmed" value={confirmed} complete={confirmed + executed > 0} />
            <DepositStageRow label="Executed" value={executed} complete={executed > 0} />
            <DepositStageRow label="Finalized" value={finalized} complete={finalized > 0} />
          </div>
          <div className="grid grid-cols-2 border-t border-white/8">
            <DepositCompactMetric label="Orphaned" value={orphaned} />
            <DepositCompactMetric
              label="Latest range"
              value={formatDepositRange(latestDepositBatch)}
            />
          </div>
        </Panel>
      </div>

      <Panel className="min-w-0 overflow-hidden">
        <SectionTitle icon={<Database className="h-4 w-4" />} title="Canonical deposits" action="L1 to L2 state input" />
        {deposits.length === 0 ? (
          <div className="px-4 py-10 text-sm text-white/48">No deposits have been indexed yet.</div>
        ) : (
          <div className="divide-y divide-white/8">
            {deposits.map((deposit) => {
              const batch = batchForDeposit(deposit, batches);
              return (
                <DepositRow
                  key={deposit.id}
                  deposit={deposit}
                  batch={batch}
                  onSelectBatch={batch ? onSelectBatch : undefined}
                />
              );
            })}
          </div>
        )}
      </Panel>
    </div>
  );
}

function DepositStageRow({ label, value, complete }: { label: string; value: number; complete: boolean }) {
  return (
    <div className="grid grid-cols-[auto_1fr_auto] items-center gap-3 rounded-md border border-white/8 bg-white/[0.035] px-3 py-2.5">
      <div className={cn(
        "flex h-7 w-7 items-center justify-center rounded-full border",
        complete ? "border-mint/30 bg-mint/10 text-mint" : "border-white/10 bg-white/[0.04] text-white/34",
      )}>
        {complete ? <CheckCircle2 className="h-4 w-4" /> : <Clock3 className="h-4 w-4" />}
      </div>
      <span className="text-sm font-medium text-white/76">{label}</span>
      <span className="text-sm font-semibold text-white">{value}</span>
    </div>
  );
}

function DepositCompactMetric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="min-w-0 border-r border-white/8 p-4 last:border-r-0">
      <div className="text-xs uppercase tracking-[0.16em] text-white/36">{label}</div>
      <div className="mt-2 truncate text-sm font-semibold text-white">{value}</div>
    </div>
  );
}

function DepositRow({
  deposit,
  batch,
  onSelectBatch,
}: {
  deposit: L1Deposit;
  batch?: RollupBatchSummary;
  onSelectBatch?: (batchNumber: number) => void;
}) {
  return (
    <div className="grid gap-3 px-4 py-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span className="shrink-0 rounded-md border border-white/10 bg-white/[0.05] px-2.5 py-1 text-sm font-semibold text-white">
            #{deposit.deposit_id}
          </span>
          <Badge tone={depositStatusTone(deposit.status)}>{depositStatusLabel(deposit.status)}</Badge>
        </div>
        <div className="min-w-0 text-right">
          <div className="text-sm font-semibold text-white">{formatDepositAmount(deposit.amount)}</div>
          <div className="mt-0.5 text-xs text-white/38">{deposit.confirmations} confirmations</div>
        </div>
      </div>
      <div className="grid gap-3 rounded-md border border-white/8 bg-white/[0.025] p-3 text-xs lg:grid-cols-[1fr_1fr_auto]">
        <AddressSummary label="L1 depositor" value={deposit.depositor_l1_address} />
        <AddressSummary label="L2 recipient" value={deposit.recipient_l2_address} />
        <div className="min-w-0">
          <div className="mb-1 text-[0.68rem] uppercase tracking-[0.14em] text-white/34">Batch</div>
          {batch ? (
            <button
              onClick={() => onSelectBatch?.(batch.batch_number)}
              className="inline-flex h-7 items-center gap-1.5 rounded-md border border-mint/20 bg-mint/10 px-2 text-xs font-medium text-mint transition hover:border-mint/40"
            >
              <PackageCheck className="h-3.5 w-3.5" />
              {batch.batch_number}
            </button>
          ) : (
            <span className="text-sm text-white/42">pending</span>
          )}
        </div>
      </div>
      <div className="grid gap-2 text-xs text-white/40 sm:grid-cols-[1fr_auto]">
        <span className="hash max-w-full">tx {shortHash(deposit.l1_tx_hash, 12, 10)}</span>
        <span>block {deposit.l1_block_number}</span>
      </div>
    </div>
  );
}

function AddressSummary({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 text-[0.68rem] uppercase tracking-[0.14em] text-white/34">{label}</div>
      <div className="hash max-w-full text-sm text-white/66" title={value}>{shortHash(value, 10, 8)}</div>
    </div>
  );
}

function DepositActionPanel({
  wallet,
  onWalletChange,
  onDeposited,
}: {
  wallet: string | null;
  onWalletChange: (wallet: string | null) => void;
  onDeposited: () => Promise<void>;
}) {
  const [recipient, setRecipient] = useState("");
  const [amount, setAmount] = useState("0.01");
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<{ tone: "good" | "bad" | "neutral"; message: string } | null>(null);
  const configured = Boolean(l1DepositInboxAddress);

  useEffect(() => {
    if (wallet && !recipient.trim()) setRecipient(wallet);
  }, [wallet, recipient]);

  async function submitDeposit() {
    if (!wallet) return;
    setSubmitting(true);
    setNotice(null);
    try {
      const txHash = await depositToL2(wallet, recipient.trim(), amount);
      setNotice({
        tone: "good",
        message: `Deposit sent: ${shortHash(txHash, 10, 8)}. It will appear here after the operator indexes the L1 event.`,
      });
      await onDeposited();
    } catch (error) {
      setNotice({ tone: "bad", message: error instanceof Error ? error.message : "Deposit failed" });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Panel className="overflow-hidden">
      <div className="border-b border-white/8 bg-white/[0.025] p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-lg font-semibold text-white">Add funds</div>
            <div className="mt-1 text-sm text-white/48">Deposit ETH from your L1 wallet to an L2 recipient.</div>
          </div>
          {wallet && <Badge tone="good">Connected</Badge>}
        </div>
      </div>

      <div className="grid gap-4 p-4">
        {!configured && (
          <div className="rounded-lg border border-amber/25 bg-amber/10 p-3 text-sm text-amber">
            Deposit contract is not configured for this UI.
          </div>
        )}

        {!wallet ? (
          <div className="rounded-lg border border-white/10 bg-white/[0.035] p-4 text-sm text-white/58">
            Connect your Ethereum wallet from the header to make a deposit.
          </div>
        ) : (
          <>
            <div className="rounded-lg border border-white/10 bg-white/[0.035] p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-xs uppercase tracking-[0.14em] text-white/38">From</div>
                  <div className="hash mt-1 max-w-full text-xs text-white/78 sm:text-sm" title={wallet}>{wallet}</div>
                </div>
                <button
                  onClick={() => onWalletChange(null)}
                  className="shrink-0 text-sm text-white/46 hover:text-white"
                >
                  Disconnect
                </button>
              </div>
            </div>

            <label className="grid gap-2 text-sm font-medium text-white/70">
              L2 recipient
              <input
                value={recipient}
                onChange={(event) => setRecipient(event.target.value)}
                placeholder="0x..."
                className="h-11 rounded-lg border border-white/10 bg-white/[0.06] px-4 font-mono text-sm text-white outline-none placeholder:text-white/30 focus:border-mint/50 focus:ring-2 focus:ring-mint/10"
              />
            </label>
            <label className="grid gap-2 text-sm font-medium text-white/70">
              Amount
              <div className="grid grid-cols-[1fr_auto] overflow-hidden rounded-lg border border-white/10 bg-white/[0.06] focus-within:border-mint/50 focus-within:ring-2 focus-within:ring-mint/10">
                <input
                  inputMode="decimal"
                  value={amount}
                  onChange={(event) => setAmount(event.target.value)}
                  placeholder="0.01"
                  className="h-11 min-w-0 bg-transparent px-4 text-lg text-white outline-none placeholder:text-white/30"
                />
                <div className="flex h-11 items-center border-l border-white/10 px-4 text-sm text-white/48">ETH</div>
              </div>
            </label>

            <button
              onClick={submitDeposit}
              disabled={!configured || submitting || recipient.trim().length === 0 || amount.trim().length === 0}
              className="inline-flex h-11 items-center justify-center gap-2 rounded-lg bg-mint px-5 text-sm font-semibold text-ink shadow-glow transition hover:bg-mint/90 disabled:cursor-not-allowed disabled:opacity-45"
            >
              <Upload className="h-4 w-4" />
              {submitting ? "Depositing..." : "Deposit"}
            </button>
          </>
        )}

        {notice && (
          <div className={cn(
            "rounded-lg border p-3 text-sm",
            notice.tone === "bad"
              ? "border-rose/25 bg-rose/10 text-rose"
              : notice.tone === "good"
                ? "border-mint/25 bg-mint/10 text-mint"
                : "border-white/10 bg-white/[0.04] text-white/64",
          )}>
            {notice.message}
          </div>
        )}
      </div>
    </Panel>
  );
}

function RollupView({
  batches,
  batchTotal,
  selectedBatchNumber,
  selectedBatch,
  batchLoading,
  onSelectBatch,
  onSelectTx,
}: {
  batches: RollupBatchSummary[];
  batchTotal: number;
  selectedBatchNumber: number | null;
  selectedBatch?: RollupBatch;
  batchLoading: boolean;
  onSelectBatch: (batchNumber: number) => void;
  onSelectTx: (hash: string) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[24rem_1fr]">
      <Panel className="overflow-hidden">
        <SectionTitle icon={<PackageCheck className="h-4 w-4" />} title="Settlement packages" action={`${batches.length}/${batchTotal} built`} />
        <div className="max-h-[36rem] overflow-auto">
          {batches.length === 0 ? (
            <div className="px-4 py-10 text-sm text-white/48">No activity groups are ready for settlement yet.</div>
          ) : (
            batches.map((batch) => (
              <button
                key={batch.batch_number}
                onClick={() => onSelectBatch(batch.batch_number)}
                className={cn(
                  "w-full border-b border-white/8 px-4 py-3 text-left transition",
                  selectedBatchNumber === batch.batch_number ? "bg-mint/10" : "hover:bg-white/[0.045]",
                )}
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="text-sm font-semibold text-white">Package {batch.batch_number}</div>
                  <Badge tone={settlementStatusTone(batch.status)}>{settlementStatusLabel(batch.status)}</Badge>
                </div>
                <div className="mt-2 text-xs text-white/46">
                  groups {batch.start_block_height}-{batch.end_block_height} / {pluralize(batch.transaction_count, "transfer")} / {pluralize(batch.deposit_count, "deposit")}
                </div>
                <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-white/48">
                  <span>{batch.l1_chain_id ? `chain ${batch.l1_chain_id}` : "L1 not selected"}</span>
                  <span>{settlementMilestoneText(batch)}</span>
                </div>
                <div className="mt-2 text-xs text-mint/72">
                  {batch.l1_finalize_tx_hash
                    ? `final ${shortHash(batch.l1_finalize_tx_hash, 10, 8)}`
                    : batch.l1_submit_tx_hash
                      ? `sent ${shortHash(batch.l1_submit_tx_hash, 10, 8)}`
                      : "not sent to L1"}
                </div>
                <div className="hash mt-2 text-xs text-white/48">{shortHash(batch.data_hash, 10, 8)}</div>
              </button>
            ))
          )}
        </div>
      </Panel>

      <Panel className="p-4">
        <SectionTitle
          icon={<Layers3 className="h-4 w-4" />}
          title="Settlement package detail"
          action={selectedBatch ? `package ${selectedBatch.batch_number}` : "none"}
          flush
        />
        {batchLoading ? (
          <Skeleton className="mt-4 h-80" />
        ) : selectedBatch ? (
          <div className="mt-4 grid gap-4">
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <MetricLine label="First group" value={selectedBatch.start_block_height} />
              <MetricLine label="Last group" value={selectedBatch.end_block_height} />
              <MetricLine label="Transfers" value={selectedBatch.transaction_count} />
            </div>
            <SettlementProgress batch={selectedBatch} />
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <MetricLine label="Deposits" value={selectedBatch.deposit_count} />
              <MetricLine label="Deposit range" value={formatDepositRange(selectedBatch)} />
              <MetricLine label="Proof input" value={selectedBatch.deposit_count > 0 ? "Bound" : "Empty"} />
            </div>
            <div className="flex flex-col gap-2 border-b border-white/8 pb-3 sm:flex-row sm:items-end sm:justify-between">
              <div>
                <h3 className="text-sm font-semibold text-white">Validity commitments</h3>
                <p className="mt-1 text-xs leading-5 text-white/48">
                  These are the public inputs checked by the proof and settlement contract.
                </p>
              </div>
            </div>
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <HashLine label="Previous state root" value={selectedBatch.prev_state_root} />
              <HashLine label="New state root" value={selectedBatch.new_state_root} />
              <HashLine label="Transaction root" value={selectedBatch.transaction_root} />
              <HashLine label="Block hash root" value={selectedBatch.block_hash_root} />
              <HashLine label="Deposit root" value={selectedBatch.deposit_root} />
              <HashLine label="Batch data hash" value={selectedBatch.data_hash} />
            </div>
            {selectedBatch.status === "built" ? (
              <div className="flex flex-col gap-3 rounded-lg border border-amber/20 bg-amber/[0.06] p-4 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <div className="text-sm font-semibold text-amber">This package is ready to prove</div>
                  <div className="mt-1 text-sm text-white/52">Open the dedicated proof workspace to generate and submit its validity proof.</div>
                </div>
                <span className="text-xs text-white/38">Select Prove in the navigation</span>
              </div>
            ) : (
              <>
                <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                  <MetricLine label="Settlement chain" value={selectedBatch.l1_chain_id ?? "unavailable"} />
                  <MetricLine label="Submit block" value={selectedBatch.l1_submit_block_number ?? "pending"} />
                  <MetricLine label="Submit gas" value={selectedBatch.l1_submit_gas_used ?? "pending"} />
                </div>
                <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                  <HashLine label="Settlement contract" value={selectedBatch.l1_contract_address} />
                  <HashLine label="Submit transaction" value={selectedBatch.l1_submit_tx_hash} />
                  {selectedBatch.status === "finalized" && (
                    <>
                      <HashLine label="Finalize transaction" value={selectedBatch.l1_finalize_tx_hash} />
                      <HashLine label="Finalized at" value={selectedBatch.l1_finalized_at} />
                    </>
                  )}
                </div>
                <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
                  <SettlementReceiptPanel
                    title="Submit receipt"
                    timestamp={selectedBatch.l1_submitted_at}
                    txHash={selectedBatch.l1_submit_tx_hash}
                    blockNumber={selectedBatch.l1_submit_block_number}
                    gasUsed={selectedBatch.l1_submit_gas_used}
                    receipt={selectedBatch.l1_submit_receipt}
                  />
                  {selectedBatch.status === "finalized" && (
                    <SettlementReceiptPanel
                      title="Finalize receipt"
                      timestamp={selectedBatch.l1_finalized_at}
                      txHash={selectedBatch.l1_finalize_tx_hash}
                      blockNumber={selectedBatch.l1_finalize_block_number}
                      gasUsed={selectedBatch.l1_finalize_gas_used}
                      receipt={selectedBatch.l1_finalize_receipt}
                    />
                  )}
                </div>
              </>
            )}
            <div className="overflow-hidden rounded-lg border border-white/10">
              <div className="grid grid-cols-[auto_1fr_auto] gap-3 border-b border-white/10 bg-white/[0.045] px-3 py-2 text-xs uppercase tracking-[0.14em] text-white/38">
                <span>Group</span>
                <span>Transfer</span>
                <span>Open</span>
              </div>
              {selectedBatch.transactions.length === 0 ? (
                <div className="px-3 py-6 text-sm text-white/48">This package has no transfers.</div>
              ) : (
                selectedBatch.transactions.map((transaction) => (
                  <div
                    key={transaction.hash}
                    className="grid grid-cols-[auto_1fr_auto] items-center gap-3 border-b border-white/8 px-3 py-3 text-sm last:border-b-0"
                  >
                    <span className="rounded-md bg-white/[0.06] px-2 py-1 text-xs font-semibold text-white/72">
                      {transaction.block_height ?? "pending"}
                    </span>
                    <span className="hash min-w-0 text-white/72">{transaction.hash}</span>
                    <button
                      onClick={() => onSelectTx(transaction.hash)}
                      className="inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.045] px-2 py-1 text-xs text-white/58 transition hover:border-mint/30 hover:text-mint"
                    >
                      <ExternalLink className="h-3 w-3" />
                      View
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>
        ) : (
          <div className="mt-4 text-sm text-white/50">Select a settlement package from the list.</div>
        )}
      </Panel>
    </div>
  );
}

function SettlementProgress({ batch }: { batch: RollupBatchSummary }) {
  const submitted = batch.status === "submitted" || batch.status === "finalized";
  const finalized = batch.status === "finalized";
  const steps = [
    { label: "Batch built", complete: true, detail: `Package ${batch.batch_number}` },
    {
      label: "Deposits bound",
      complete: batch.deposit_count === 0 || batch.deposit_root !== zeroHash(),
      detail: batch.deposit_count > 0 ? formatDepositRange(batch) : "Empty range",
    },
    { label: "Proof submitted", complete: submitted, detail: submitted ? "Validity proof sent" : "Next action" },
    { label: "L1 finalized", complete: finalized, detail: finalized ? `Chain ${batch.l1_chain_id}` : "Waiting" },
  ];

  return (
    <div className="rounded-lg border border-white/10 bg-black/15 p-3">
      <div className="mb-3 flex items-center justify-between gap-3">
        <span className="text-xs font-medium uppercase tracking-[0.16em] text-white/38">Settlement progress</span>
        <Badge tone={settlementStatusTone(batch.status)}>{settlementStatusLabel(batch.status)}</Badge>
      </div>
      <div className="grid gap-2 md:grid-cols-4">
        {steps.map((step, index) => (
          <div key={step.label} className="flex items-center gap-3 rounded-lg border border-white/8 bg-white/[0.035] p-3">
            <div className={cn(
              "flex h-7 w-7 shrink-0 items-center justify-center rounded-full border text-xs font-semibold",
              step.complete
                ? "border-mint/30 bg-mint/10 text-mint"
                : "border-white/10 bg-white/[0.04] text-white/38",
            )}>
              {step.complete ? <CheckCircle2 className="h-4 w-4" /> : index + 1}
            </div>
            <div>
              <div className="text-sm font-semibold text-white">{step.label}</div>
              <div className="mt-0.5 text-xs text-white/44">{step.detail}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function ProofWorkspace({
  batches,
  selectedBatchNumber,
  wallet,
  onSelectBatch,
  onSubmitted,
}: {
  batches: RollupBatchSummary[];
  selectedBatchNumber: number | null;
  wallet: string | null;
  onSelectBatch: (batchNumber: number) => void;
  onSubmitted: (batchNumber: number) => Promise<void>;
}) {
  const readyBatches = batches.filter((batch) => batch.status === "built");
  const batch = readyBatches.find((candidate) => candidate.batch_number === selectedBatchNumber) ?? readyBatches[0];

  if (!batch) {
    return (
      <Panel className="p-8 text-center">
        <PackageCheck className="mx-auto h-7 w-7 text-white/30" />
        <div className="mt-3 font-semibold text-white">No batches are ready to prove</div>
        <div className="mt-1 text-sm text-white/48">A proof workspace will appear after the operator builds a settlement batch.</div>
      </Panel>
    );
  }

  return (
    <Panel className="p-4">
      <div className="mb-4 flex flex-col gap-3 border-b border-white/8 pb-4 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <div className="text-xs font-medium uppercase tracking-[0.14em] text-white/38">Ready batch</div>
          <div className="mt-1 text-lg font-semibold text-white">Package {batch.batch_number}</div>
        </div>
        <label className="grid gap-1.5 text-xs text-white/46">
          Select package
          <select
            value={batch.batch_number}
            onChange={(event) => onSelectBatch(Number(event.target.value))}
            className="h-10 min-w-48 rounded-md border border-white/12 bg-ink px-3 text-sm text-white outline-none focus:border-mint/40"
          >
            {readyBatches.map((candidate) => (
              <option key={candidate.batch_number} value={candidate.batch_number}>
                Package {candidate.batch_number} · {candidate.transaction_count} transfers
              </option>
            ))}
          </select>
        </label>
      </div>
      <SettlementNextStep
        batch={batch}
        wallet={wallet}
        onSubmitted={() => onSubmitted(batch.batch_number)}
      />
    </Panel>
  );
}

function SettlementNextStep({
  batch,
  wallet,
  onSubmitted,
}: {
  batch: RollupBatchSummary;
  wallet: string | null;
  onSubmitted: () => Promise<void>;
}) {
  const [artifactText, setArtifactText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<{ tone: "good" | "bad" | "neutral"; message: string } | null>(null);
  const witnessUrl = `${API_BASE}/rollup/batches/${batch.batch_number}/validity-witness`;
  const calldataUrl = `${API_BASE}/rollup/batches/${batch.batch_number}/settlement-calldata`;
  const canSubmit = Boolean(wallet && settlementContractAddress && artifactText.trim().length > 0);

  async function loadArtifact(file: File | undefined) {
    if (!file) return;
    setArtifactText(await file.text());
    setNotice({ tone: "neutral", message: `${file.name} loaded. Review it, then submit the proof transaction.` });
  }

  async function submitProof() {
    if (!wallet) return;
    setSubmitting(true);
    setNotice(null);
    try {
      const artifact = parseProofArtifact(artifactText);
      const txHash = await submitBatchWithProof(wallet, batch, artifact);
      setNotice({ tone: "good", message: `Proof transaction sent: ${shortHash(txHash, 12, 10)}.` });
      await onSubmitted();
    } catch (error) {
      setNotice({ tone: "bad", message: error instanceof Error ? error.message : "Proof submission failed" });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="rounded-lg border border-amber/25 bg-amber/10 p-4">
      <div className="grid gap-4">
        <div>
          <div className="text-sm font-semibold text-amber">Next: submit the validity proof</div>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-white/58">
            Generate the proof from this package's witness, then load the artifact here to submit it from your wallet.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <a
              href={witnessUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-9 items-center gap-2 rounded-lg border border-white/12 bg-white/[0.05] px-3 text-sm font-medium text-white/72 hover:border-mint/30 hover:text-mint"
            >
              <ExternalLink className="h-4 w-4" />
              Open witness
            </a>
            <a
              href={calldataUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-9 items-center gap-2 rounded-lg border border-white/12 bg-white/[0.05] px-3 text-sm font-medium text-white/72 hover:border-mint/30 hover:text-mint"
            >
              <ExternalLink className="h-4 w-4" />
              Open calldata
            </a>
          </div>
        </div>

        <div className="grid gap-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-sm text-white/62">Proof artifact JSON</span>
            <label className="inline-flex h-9 cursor-pointer items-center justify-center gap-2 rounded-md border border-white/12 bg-white/[0.05] px-3 text-sm font-medium text-white/72 transition hover:border-mint/30 hover:text-mint">
              <Upload className="h-4 w-4" />
              Load artifact
              <input
                type="file"
                accept="application/json,.json"
                className="hidden"
                onChange={(event) => void loadArtifact(event.target.files?.[0])}
              />
            </label>
          </div>
          <label>
            <span className="sr-only">Proof artifact JSON</span>
            <textarea
              value={artifactText}
              onChange={(event) => setArtifactText(event.target.value)}
              placeholder='{"public_values":"0x...","proof":"0x..."}'
              className="min-h-32 w-full resize-y rounded-lg border border-white/10 bg-black/20 p-3 font-mono text-xs leading-5 text-white outline-none placeholder:text-white/28 focus:border-mint/40"
            />
          </label>
        </div>

        <div className="flex flex-col gap-3 border-t border-white/10 pt-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-white/46">
            <span>{settlementContractAddress ? `contract ${shortHash(settlementContractAddress, 8, 6)}` : "settlement contract not configured"}</span>
            <span>{wallet ? `wallet ${shortHash(wallet, 8, 6)}` : "connect wallet from the header"}</span>
            <span>{artifactText.trim() ? "artifact loaded" : "waiting for artifact"}</span>
          </div>
          <button
            onClick={submitProof}
            disabled={submitting || !canSubmit}
            className="inline-flex h-10 shrink-0 items-center justify-center gap-2 rounded-md bg-amber px-4 text-sm font-semibold text-ink transition hover:bg-amber/90 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/30"
          >
            <ShieldAlert className="h-4 w-4" />
            {submitting ? "Submitting" : "Submit proof"}
          </button>
        </div>

        {notice && (
          <div
            className={cn(
              "rounded-lg border px-3 py-2 text-sm font-medium",
              notice.tone === "bad"
                ? "border-rose/25 bg-rose/10 text-rose"
                : notice.tone === "good"
                  ? "border-mint/25 bg-mint/10 text-mint"
                  : "border-white/10 bg-white/[0.05] text-white/64",
            )}
          >
            {notice.message}
          </div>
        )}
      </div>
    </div>
  );
}

function SettlementReceiptPanel({
  title,
  timestamp,
  txHash,
  blockNumber,
  gasUsed,
  receipt,
}: {
  title: string;
  timestamp?: string | null;
  txHash?: string | null;
  blockNumber?: number | null;
  gasUsed?: number | null;
  receipt?: unknown | null;
}) {
  const hasReceipt = Boolean(receipt);

  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-sm font-semibold text-white">
          <ReceiptText className="h-4 w-4 text-mint" />
          {title}
        </div>
        <Badge tone={hasReceipt ? "good" : "neutral"}>{hasReceipt ? "Indexed" : "Missing"}</Badge>
      </div>
      <div className="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3">
        <ReceiptMetric icon={<Clock3 className="h-3.5 w-3.5" />} label="Time" value={timestamp ?? "none"} />
        <ReceiptMetric icon={<Blocks className="h-3.5 w-3.5" />} label="Block" value={blockNumber ?? "none"} />
        <ReceiptMetric icon={<Gauge className="h-3.5 w-3.5" />} label="Gas" value={gasUsed ?? "none"} />
      </div>
      <div className="mt-3 flex min-w-0 items-center gap-2 rounded-md border border-white/8 bg-black/15 px-2.5 py-2">
        <span className="text-xs uppercase tracking-[0.14em] text-white/36">Tx</span>
        <span className="hash min-w-0 flex-1 text-sm text-white/72">{txHash ?? "none"}</span>
        {txHash && <CopyButton value={txHash} label={`${title} transaction`} />}
      </div>
    </div>
  );
}

function ReceiptMetric({ icon, label, value }: { icon: React.ReactNode; label: string; value: React.ReactNode }) {
  return (
    <div className="min-w-0 rounded-md border border-white/8 bg-black/15 px-2.5 py-2">
      <div className="flex items-center gap-1.5 text-xs uppercase tracking-[0.14em] text-white/36">
        <span className="text-white/42">{icon}</span>
        {label}
      </div>
      <div className="mt-1 truncate text-sm font-medium text-white/78">{value}</div>
    </div>
  );
}

function SectionTitle({ icon, title, action, flush = false }: { icon: React.ReactNode; title: string; action?: React.ReactNode; flush?: boolean }) {
  return (
    <div className={cn("flex items-center justify-between gap-3", !flush && "border-b border-white/10 px-4 py-3")}>
      <div className="flex items-center gap-2 text-sm font-semibold text-white">
        <span className="text-mint">{icon}</span>
        {title}
      </div>
      {action && <div className="text-xs text-white/42">{action}</div>}
    </div>
  );
}

function HashLine({ label, value }: { label: string; value?: string | null }) {
  return (
    <div className="min-w-0 rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="mb-2 text-xs uppercase tracking-[0.16em] text-white/36">{label}</div>
      <div className="flex min-w-0 items-center gap-2">
        <div className="hash min-w-0 flex-1 text-sm text-white/78">{value ?? "none"}</div>
        {value && <CopyButton value={value} label={label} />}
      </div>
    </div>
  );
}

function settlementStatusTone(status: string): "neutral" | "good" | "warn" | "bad" {
  if (status === "finalized") return "good";
  if (status === "submitted") return "warn";
  if (status === "rejected" || status === "challenged") return "bad";
  return "neutral";
}

function settlementStatusLabel(status: string) {
  if (status === "built") return "Ready";
  if (status === "submitted") return "Sent";
  if (status === "finalized") return "Final";
  if (status === "challenged") return "Under review";
  if (status === "rejected") return "Rejected";
  return status;
}

function depositStatusTone(status: string): "neutral" | "good" | "warn" | "bad" {
  if (status === "finalized" || status === "executed") return "good";
  if (status === "confirmed") return "warn";
  if (status === "orphaned") return "bad";
  return "neutral";
}

function depositStatusLabel(status: string) {
  if (status === "observed") return "Observed";
  if (status === "confirmed") return "Confirmed";
  if (status === "executed") return "Executed";
  if (status === "finalized") return "Final";
  if (status === "orphaned") return "Orphaned";
  return status;
}

function formatDepositAmount(amountWei: number) {
  if (!Number.isFinite(amountWei) || amountWei <= 0) return "0 ETH";
  const wei = BigInt(Math.trunc(amountWei));
  const base = 10n ** 18n;
  const whole = wei / base;
  const fraction = wei % base;
  if (fraction === 0n) return `${whole.toString()} ETH`;
  const fractionText = fraction.toString().padStart(18, "0").replace(/0+$/, "").slice(0, 6);
  return `${whole.toString()}.${fractionText} ETH`;
}

function batchForDeposit(deposit: L1Deposit, batches: RollupBatchSummary[]) {
  return batches.find((batch) => {
    if (batch.first_deposit_id === null || batch.last_deposit_id === null) return false;
    return deposit.deposit_id >= batch.first_deposit_id && deposit.deposit_id <= batch.last_deposit_id;
  });
}

function confirmedDepositCount(deposits: L1Deposit[]) {
  return deposits.filter((deposit) => deposit.status === "confirmed").length;
}

function formatDepositRange(batch?: RollupBatchSummary | null) {
  if (!batch || batch.deposit_count === 0 || batch.first_deposit_id === null || batch.last_deposit_id === null) {
    return "none";
  }
  if (batch.first_deposit_id === batch.last_deposit_id) return `#${batch.first_deposit_id}`;
  return `#${batch.first_deposit_id}-#${batch.last_deposit_id}`;
}

function zeroHash() {
  return `0x${"0".repeat(64)}`;
}

function settlementMilestoneText(batch: RollupBatchSummary) {
  if (batch.l1_finalized_at) return "finalized";
  if (batch.l1_submitted_at) return "submitted";
  return "ready";
}

function MetricLine({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="mb-2 text-xs uppercase tracking-[0.16em] text-white/36">{label}</div>
      <div className="break-words text-xl font-semibold leading-tight text-white sm:text-2xl">{value}</div>
    </div>
  );
}

function CopyButton({ value, label = "value" }: { value: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      title={`Copy ${label.toLowerCase()}`}
      aria-label={`Copy ${label.toLowerCase()}`}
      onClick={async () => {
        await navigator.clipboard.writeText(value);
        setCopied(true);
        window.setTimeout(() => setCopied(false), 900);
      }}
      className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-white/44 transition hover:bg-white/10 hover:text-mint"
    >
      {copied ? <CheckCircle2 className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
    </button>
  );
}

function downloadTransactionReference({
  hash,
  transfer,
  blockHeight,
  transactionIndex,
  batch,
}: {
  hash: string;
  transfer: Transfer;
  blockHeight: number | null;
  transactionIndex: number | null;
  batch?: RollupBatchSummary;
}) {
  const apiBase = API_BASE.startsWith("/")
    ? `${window.location.origin}${API_BASE}`
    : API_BASE;
  const reference: TransactionReferenceFile = {
    format: "rolluplens.transaction-reference",
    version: 1,
    exported_at: new Date().toISOString(),
    api_base: apiBase,
    transaction: {
      hash,
      block_height: blockHeight,
      transaction_index: transactionIndex,
      from: transfer.from,
      to: transfer.to,
      amount: transfer.amount,
      nonce: transfer.nonce,
    },
    batch: batch ? { number: batch.batch_number, status: batch.status } : null,
  };
  const blob = new Blob([`${JSON.stringify(reference, null, 2)}\n`], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `rolluplens-transaction-${hash.slice(0, 12)}.json`;
  anchor.click();
  URL.revokeObjectURL(url);
}

function TransactionRow({
  tx,
  hash,
  onSelectAccount,
  onSelectTx,
}: {
  tx: Transfer;
  hash: string | null;
  onSelectAccount: (address: string) => void;
  onSelectTx: (hash: string) => void;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_5rem_minmax(0,1fr)_4rem] items-center gap-3 border-b border-white/8 px-3 py-3 text-sm last:border-b-0">
      <button
        onClick={() => onSelectAccount(tx.from)}
        title={tx.from}
        className="hash min-w-0 text-center text-cyan hover:text-cyan/80"
      >
        {shortHash(tx.from, 10, 8)}
      </button>
      <span className="justify-self-center rounded-md bg-amber/10 px-2 py-1 text-xs font-semibold text-amber">{tx.amount}</span>
      <button
        onClick={() => onSelectAccount(tx.to)}
        title={tx.to}
        className="hash min-w-0 text-center text-mint hover:text-mint/80"
      >
        {shortHash(tx.to, 10, 8)}
      </button>
      <button
        disabled={!hash}
        onClick={() => hash && onSelectTx(hash)}
        title={hash ? `Open transaction ${hash}` : "Transaction unavailable"}
        aria-label={hash ? "Open transaction details" : "Transaction unavailable"}
        className="inline-flex h-8 w-8 items-center justify-center justify-self-center rounded-md border border-white/10 bg-white/[0.045] text-white/58 transition hover:border-mint/30 hover:text-mint disabled:opacity-40"
      >
        <ExternalLink className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function TransferDetail({ tx, onSelectAccount }: { tx: Transfer; onSelectAccount: (address: string) => void }) {
  return (
    <div className="grid gap-3 md:col-span-2 md:grid-cols-2">
      <HashLine label="Sender wallet" value={tx.from} />
      <HashLine label="Receiver wallet" value={tx.to} />
      <MetricLine label="Amount" value={tx.amount} />
      <MetricLine label="Transfer #" value={tx.nonce} />
      <button onClick={() => onSelectAccount(tx.from)} className="rounded-lg border border-cyan/20 bg-cyan/10 p-3 text-left text-sm font-medium text-cyan">
        Open sender
      </button>
      <button onClick={() => onSelectAccount(tx.to)} className="rounded-lg border border-mint/20 bg-mint/10 p-3 text-left text-sm font-medium text-mint">
        Open receiver
      </button>
    </div>
  );
}
