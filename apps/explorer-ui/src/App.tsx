import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  ArrowRight,
  ArrowUpRight,
  BarChart3,
  Blocks,
  CheckCircle2,
  Copy,
  Cpu,
  Database,
  ExternalLink,
  Gauge,
  Landmark,
  Layers3,
  Network,
  PackageCheck,
  Play,
  RefreshCcw,
  Search,
  Send,
  ShieldAlert,
  Users,
  Wallet,
  XCircle,
} from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import {
  API_BASE,
  api,
  type Account,
  type Block,
  type BlockSummary,
  type RollupBatch,
  type RollupBatchSummary,
  type SearchResult,
  type Transfer,
  type TransactionPoolEntry,
} from "./lib/api";
import { cn, formatNumber, pluralize, shortHash } from "./lib/utils";
import { Badge, IconButton, Panel, Shell, Skeleton, Stat } from "./components/ui";

const tabs = ["Overview", "Blocks", "Accounts", "Rollup", "Sequencer", "Health"] as const;
type Tab = (typeof tabs)[number];

const tabMeta: Record<Tab, { label: string; icon: React.ReactNode }> = {
  Overview: { label: "Dashboard", icon: <Gauge className="h-4 w-4" /> },
  Blocks: { label: "Activity", icon: <Blocks className="h-4 w-4" /> },
  Accounts: { label: "People", icon: <Users className="h-4 w-4" /> },
  Rollup: { label: "Settlement", icon: <PackageCheck className="h-4 w-4" /> },
  Sequencer: { label: "Send", icon: <Send className="h-4 w-4" /> },
  Health: { label: "System", icon: <ShieldAlert className="h-4 w-4" /> },
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

  const summary = useQuery({ queryKey: ["summary"], queryFn: api.summary });
  const blocks = useQuery({ queryKey: ["blocks"], queryFn: () => api.blocks() });
  const health = useQuery({ queryKey: ["health"], queryFn: api.health });
  const errors = useQuery({ queryKey: ["errors"], queryFn: api.errors });
  const rollupBatches = useQuery({ queryKey: ["rollup-batches"], queryFn: () => api.rollupBatches() });
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
  const connectionError = firstError(summary.error, blocks.error);
  const connectionMessage = connectionError ?? "Backend request failed";
  const hasCoreData = Boolean(summary.data && blocks.data);
  const isOffline = Boolean(connectionError && !hasCoreData);
  const isStale = Boolean(connectionError && hasCoreData);
  const readStatus = isOffline ? "offline" : isStale ? "stale" : hasCoreData ? "ok" : "connecting";
  const blockItems = blocks.data?.items ?? [];
  const blockTotal = blocks.data?.total ?? blockItems.length;
  const batchItems = rollupBatches.data?.items ?? [];
  const batchTotal = rollupBatches.data?.total ?? batchItems.length;

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

  async function onSearch(query: string) {
    const trimmed = query.trim();
    if (!trimmed) return;
    setSearchError(null);
    try {
      const result = await api.search(trimmed);
      setSearchResult(result);
      if (result.kind === "Block") {
        setSelectedHeight(result.data.height);
        setActiveTab("Blocks");
      }
      if (result.kind === "Account") {
        setSelectedAccount(result.data.address);
        setActiveTab("Accounts");
      }
      if (result.kind === "Transaction") {
        setSelectedTx(result.data.hash);
        setActiveTab("Blocks");
      }
    } catch (error) {
      setSearchResult(null);
      setSearchError(error instanceof Error ? error.message : "Search failed");
    }
  }

  return (
    <Shell>
      <header className="sticky top-0 z-30 -mx-4 border-b border-white/10 bg-ink/82 px-4 py-3 backdrop-blur-xl sm:-mx-6 sm:px-6 lg:-mx-8 lg:px-8">
        <div className="mx-auto grid max-w-[88rem] gap-3">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="flex min-w-0 items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-mint/25 bg-mint/10 text-mint shadow-glow">
                <Layers3 className="h-5 w-5" />
              </div>
              <div className="min-w-0">
                <h1 className="truncate text-xl font-semibold leading-tight text-white sm:text-2xl">RollupLens Explorer</h1>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-white/56 sm:text-sm">
                  <Badge tone={readStatus === "ok" ? "good" : "warn"}>
                    {readStatusLabel(readStatus)}
                  </Badge>
                  <span>Transfers, accounts, and settlement</span>
                  <span className="hidden text-white/24 sm:inline">/</span>
                  <span className="hash max-w-[12rem]">{shortHash(summary.data?.latest_block_hash)}</span>
                </div>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <SearchBox onSearch={onSearch} pending={false} />
              <IconButton onClick={refreshAll} title="Refresh data" aria-label="Refresh data">
                <RefreshCcw className="h-4 w-4" />
              </IconButton>
            </div>
          </div>

          <HeaderNav activeTab={activeTab} onSelectTab={setActiveTab} />
        </div>
      </header>

      {isOffline && <ConnectionError message={connectionMessage} />}
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
        <div className="grid gap-5">
          <HomeHero
            readStatus={readStatus}
            summary={summary.data}
            latestBlock={blockItems[0]}
            pendingTransfers={summary.data?.pending_transaction_count ?? 0}
            onOpenActivity={() => setActiveTab("Blocks")}
            onOpenSettlement={() => setActiveTab("Rollup")}
          />
          <SummaryStats summary={summary.data} loading={summary.isLoading} />
          <Overview
            summary={summary.data}
            blocks={blockItems}
            blockTotal={blockTotal}
            selectedBlock={block.data}
            onSelectBlock={setSelectedHeight}
            onSelectAccount={(address) => {
              setSelectedAccount(address);
              setActiveTab("Accounts");
            }}
          />
        </div>
      )}

      {activeTab === "Blocks" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Blocks className="h-5 w-5" />}
            title="Activity"
            description="Inspect grouped transfers, block fingerprints, sender and receiver flow, and individual transfer details."
            meta={`${blockItems.length}/${blockTotal} groups loaded`}
          />
          <BlocksView
            blocks={blockItems}
            blockTotal={blockTotal}
            selectedHeight={selectedHeight}
            selectedBlock={block.data}
            blockLoading={block.isLoading}
            selectedTx={selectedTx}
            transaction={transaction.data}
            transactionError={transaction.error instanceof Error ? transaction.error.message : null}
            onSelectBlock={setSelectedHeight}
            onSelectAccount={(address) => {
              setSelectedAccount(address);
              setActiveTab("Accounts");
            }}
            onSelectTx={setSelectedTx}
          />
        </div>
      )}

      {activeTab === "Accounts" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Users className="h-5 w-5" />}
            title="People"
            description="Look up wallet balances, open senders and receivers from current activity, and keep account state separate from block browsing."
            meta={selectedAccount ? shortHash(selectedAccount) : "no wallet selected"}
          />
          <AccountsView
            candidates={accountsFromBlocks}
            selectedAccount={selectedAccount}
            account={account.data}
            accountError={account.error instanceof Error ? account.error.message : null}
            onSelectAccount={setSelectedAccount}
          />
        </div>
      )}

      {activeTab === "Rollup" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<PackageCheck className="h-5 w-5" />}
            title="Settlement"
            description="Review settlement packages, L1 receipt metadata, state commitments, and the transfers included in each package."
            meta={`${batchItems.length}/${batchTotal} packages`}
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
              setActiveTab("Blocks");
            }}
          />
        </div>
      )}

      {activeTab === "Sequencer" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<Send className="h-5 w-5" />}
            title="Send"
            description="Submit signed transfer JSON and group waiting transfers into the next local activity block."
            meta={`${transactionPool.data?.total ?? 0} waiting or processed`}
          />
          <SequencerView
            pool={transactionPool.data?.items ?? []}
            poolTotal={transactionPool.data?.total ?? 0}
            onSubmitted={async (hash) => {
              setSelectedTx(hash);
              await refreshAll();
            }}
            onSequenced={async (height) => {
              setSelectedHeight(height);
              setActiveTab("Blocks");
              await refreshAll();
            }}
          />
        </div>
      )}

      {activeTab === "Health" && (
        <div className="grid gap-5">
          <PageIntro
            icon={<ShieldAlert className="h-5 w-5" />}
            title="System"
            description="Monitor API status, indexed data consistency, and recent service events without mixing them into explorer workflows."
            meta={health.data?.status ?? "checking"}
          />
          <HealthView health={health.data} errors={errors.data ?? []} summary={summary.data} />
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
  if (status === "stale") return "Last good data";
  if (status === "offline") return "Offline";
  return "Connecting";
}

function HeaderNav({ activeTab, onSelectTab }: { activeTab: Tab; onSelectTab: (tab: Tab) => void }) {
  return (
    <nav className="flex min-w-0 flex-wrap gap-1 rounded-xl border border-white/10 bg-white/[0.035] p-1">
      {tabs.map((tab) => (
        <button
          key={tab}
          onClick={() => onSelectTab(tab)}
          className={cn(
            "relative inline-flex h-10 shrink-0 items-center gap-2 rounded-lg px-3 text-sm font-medium text-white/58 transition hover:text-white",
            activeTab === tab && "text-white",
          )}
        >
          {activeTab === tab && (
            <motion.span
              layoutId="active-header-tab"
              className="absolute inset-0 rounded-lg border border-mint/25 bg-mint/10"
              transition={{ type: "spring", bounce: 0.18, duration: 0.45 }}
            />
          )}
          <span className="relative text-mint/80">{tabMeta[tab].icon}</span>
          <span className="relative">{tabMeta[tab].label}</span>
        </button>
      ))}
    </nav>
  );
}

function SummaryStats({
  summary,
  loading,
}: {
  summary?: {
    height: number | null;
    transaction_count: number;
    pending_transaction_count: number;
    account_count: number;
    valid: boolean;
    genesis_finalized: boolean;
  };
  loading: boolean;
}) {
  return (
    <section className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-4">
      {loading ? (
        <>
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
        </>
      ) : (
        <>
          <Stat label="Latest block" value={summary?.height ?? 0} detail="Newest confirmed activity group" accent="mint" />
          <Stat label="Transfers" value={formatNumber(summary?.transaction_count)} detail={`${formatNumber(summary?.pending_transaction_count)} waiting to be grouped`} accent="cyan" />
          <Stat label="Accounts" value={formatNumber(summary?.account_count)} detail="Wallets with current balances" accent="amber" />
          <Stat label="System check" value={summary?.valid ? "Healthy" : "Needs review"} detail={summary?.genesis_finalized ? "Starting point is locked" : "Starting point is not locked"} accent={summary?.valid ? "mint" : "rose"} />
        </>
      )}
    </section>
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
    <section className="rounded-2xl border border-white/10 bg-white/[0.025] p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-mint/20 bg-mint/10 text-mint">
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

function ConnectionError({ message }: { message: string }) {
  return (
    <Panel className="border-rose/25 bg-rose/10 p-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-semibold text-rose">
            <XCircle className="h-4 w-4" />
            Backend connection failed
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
      className="flex h-10 w-full min-w-0 max-w-xl items-center gap-2 rounded-lg border border-white/10 bg-white/[0.06] px-3 focus-within:border-mint/40 focus-within:ring-2 focus-within:ring-mint/20 sm:w-[28rem]"
      onSubmit={(event) => {
        event.preventDefault();
        onSearch(value);
      }}
    >
      <Search className="h-4 w-4 shrink-0 text-white/42" />
      <input
        value={value}
        onChange={(event) => setValue(event.target.value)}
        placeholder="Search height, hash, tx, address"
        className="min-w-0 flex-1 bg-transparent text-sm text-white outline-none placeholder:text-white/34"
      />
      <button
        type="submit"
        disabled={pending}
        className="rounded-md bg-white/10 px-3 py-1.5 text-xs font-medium text-white/80 transition hover:bg-mint/15 hover:text-mint disabled:opacity-50"
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

function HomeHero({
  readStatus,
  summary,
  latestBlock,
  pendingTransfers,
  onOpenActivity,
  onOpenSettlement,
}: {
  readStatus: string;
  summary?: { height: number | null; transaction_count: number; account_count: number; valid: boolean };
  latestBlock?: BlockSummary;
  pendingTransfers: number;
  onOpenActivity: () => void;
  onOpenSettlement: () => void;
}) {
  const live = readStatus === "ok";
  return (
    <section className="relative overflow-hidden rounded-[1.4rem] border border-white/10 bg-white/[0.025] px-4 py-6 shadow-2xl shadow-black/20 sm:px-6 lg:px-8">
      <div className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-mint/70 to-transparent" />
      <div className="grid min-h-[30rem] grid-cols-1 items-center gap-8 lg:grid-cols-[0.92fr_1.08fr]">
        <div className="max-w-3xl">
          <div className="mb-5 inline-flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.045] px-3 py-1.5 text-xs font-medium text-white/68">
            <span className={cn("h-2 w-2 rounded-full", live ? "bg-mint shadow-[0_0_18px_rgba(88,244,183,0.8)]" : "bg-amber")} />
            {live ? "Live rollup monitor" : "Waiting for local services"}
          </div>
          <h2 className="max-w-4xl text-4xl font-semibold leading-[1.02] text-white sm:text-5xl lg:text-6xl">
            A settlement-aware explorer for local rollup operations.
          </h2>
          <p className="mt-5 max-w-2xl text-base leading-7 text-white/62 sm:text-lg">
            Track transfers from intake to grouped blocks, settlement packages, and L1 receipts without digging through logs or raw database rows.
          </p>
          <div className="mt-7 flex flex-col gap-3 sm:flex-row">
            <button
              onClick={onOpenActivity}
              className="inline-flex h-11 items-center justify-center gap-2 rounded-lg bg-mint px-4 text-sm font-semibold text-ink shadow-lg shadow-mint/10 transition hover:bg-mint/90"
            >
              Open activity
              <ArrowUpRight className="h-4 w-4" />
            </button>
            <button
              onClick={onOpenSettlement}
              className="inline-flex h-11 items-center justify-center gap-2 rounded-lg border border-white/12 bg-white/[0.045] px-4 text-sm font-semibold text-white/82 transition hover:border-amber/30 hover:bg-amber/10 hover:text-amber"
            >
              View settlement
              <Landmark className="h-4 w-4" />
            </button>
          </div>
          <div className="mt-8 grid grid-cols-2 gap-3 sm:grid-cols-4">
            <HeroMetric label="Latest block" value={summary?.height ?? "none"} />
            <HeroMetric label="Transfers" value={formatNumber(summary?.transaction_count)} />
            <HeroMetric label="Wallets" value={formatNumber(summary?.account_count)} />
            <HeroMetric label="Waiting" value={formatNumber(pendingTransfers)} />
          </div>
        </div>

        <div className="hero-visual relative min-h-[26rem] overflow-hidden rounded-[1.25rem] border border-white/10 p-4">
          <div className="scan-line absolute left-0 top-24 h-px w-full" />
          <div className="absolute inset-4 rounded-2xl border border-white/8" />
          <div className="absolute left-8 top-8 right-8 flex items-center justify-between text-xs text-white/42">
            <span>rollup path</span>
            <span className="hash">{shortHash(latestBlock?.block_hash, 8, 6)}</span>
          </div>
          <TopologyNode className="left-[8%] top-[34%]" icon={<Send className="h-4 w-4" />} label="Intake" value={`${pendingTransfers} waiting`} tone="mint" />
          <TopologyNode className="left-[35%] top-[18%]" icon={<Cpu className="h-4 w-4" />} label="Executor" value={summary?.valid ? "consistent" : "review"} tone="cyan" />
          <TopologyNode className="left-[57%] top-[48%]" icon={<PackageCheck className="h-4 w-4" />} label="Package" value={latestBlock ? `block ${latestBlock.height}` : "idle"} tone="amber" />
          <TopologyNode className="right-[7%] top-[28%]" icon={<Landmark className="h-4 w-4" />} label="L1 receipt" value="indexed" tone="rose" />
          <div className="absolute left-[18%] top-[43%] h-px w-[22%] rotate-[-18deg] bg-gradient-to-r from-mint/70 to-cyan/60" />
          <div className="absolute left-[45%] top-[34%] h-px w-[20%] rotate-[28deg] bg-gradient-to-r from-cyan/60 to-amber/70" />
          <div className="absolute left-[67%] top-[45%] h-px w-[22%] rotate-[-16deg] bg-gradient-to-r from-amber/70 to-rose/70" />
          <div className="absolute bottom-5 left-5 right-5 grid grid-cols-3 gap-3">
            <SignalTile icon={<Network className="h-4 w-4" />} label="Blocks" value={latestBlock ? String(latestBlock.height) : "0"} />
            <SignalTile icon={<BarChart3 className="h-4 w-4" />} label="Throughput" value={pluralize(latestBlock?.transaction_count ?? 0, "transfer")} />
            <SignalTile icon={<ShieldAlert className="h-4 w-4" />} label="Status" value={readStatusLabel(readStatus)} />
          </div>
        </div>
      </div>
    </section>
  );
}

function HeroMetric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-white/10 bg-white/[0.04] p-3">
      <div className="text-xs text-white/42">{label}</div>
      <div className="mt-2 text-xl font-semibold text-white">{value}</div>
    </div>
  );
}

function TopologyNode({
  className,
  icon,
  label,
  value,
  tone,
}: {
  className: string;
  icon: React.ReactNode;
  label: string;
  value: string;
  tone: "mint" | "cyan" | "amber" | "rose";
}) {
  const tones = {
    mint: "border-mint/25 bg-mint/10 text-mint",
    cyan: "border-cyan/25 bg-cyan/10 text-cyan",
    amber: "border-amber/25 bg-amber/10 text-amber",
    rose: "border-rose/25 bg-rose/10 text-rose",
  };
  return (
    <div className={cn("absolute min-w-32 rounded-xl border p-3 shadow-xl shadow-black/20 backdrop-blur", tones[tone], className)}>
      <div className="flex items-center gap-2 text-sm font-semibold">
        {icon}
        {label}
      </div>
      <div className="mt-2 text-xs text-white/58">{value}</div>
    </div>
  );
}

function SignalTile({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-xl border border-white/10 bg-black/20 p-3">
      <div className="flex items-center gap-2 text-xs text-white/46">
        <span className="text-mint">{icon}</span>
        {label}
      </div>
      <div className="mt-2 truncate text-sm font-semibold text-white">{value}</div>
    </div>
  );
}

function Overview({
  summary,
  blocks,
  blockTotal,
  selectedBlock,
  onSelectBlock,
  onSelectAccount,
}: {
  summary?: { state_root: string; latest_block_hash: string | null; block_count: number };
  blocks: BlockSummary[];
  blockTotal: number;
  selectedBlock?: Block;
  onSelectBlock: (height: number) => void;
  onSelectAccount: (address: string) => void;
}) {
  const transferCount = selectedBlock?.transactions.length ?? 0;
  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[1.2fr_0.8fr]">
      <div className="xl:col-span-2">
        <WorkflowStrip
          latestBlock={summary?.block_count ? summary.block_count - 1 : null}
          transferCount={transferCount}
          hasSettlement={false}
        />
      </div>
      <Panel className="overflow-hidden">
        <SectionTitle icon={<Blocks className="h-4 w-4" />} title="Recent activity groups" action={`${blocks.length}/${blockTotal} loaded`} />
        <div className="divide-y divide-white/8">
          {blocks.map((block) => (
            <button
              key={block.height}
              onClick={() => onSelectBlock(block.height)}
              className="grid w-full grid-cols-[auto_1fr_auto] items-center gap-3 px-4 py-3 text-left transition hover:bg-white/[0.045]"
            >
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-cyan/10 text-cyan">
                #{block.height}
              </div>
              <div className="min-w-0">
                <div className="hash text-sm text-white">{block.block_hash}</div>
                <div className="mt-1 text-xs text-white/45">{pluralize(block.transaction_count, "transfer")}</div>
              </div>
              <ArrowRight className="h-4 w-4 text-white/34" />
            </button>
          ))}
        </div>
      </Panel>

      <Panel className="p-4">
        <SectionTitle icon={<Gauge className="h-4 w-4" />} title="Current snapshot" action="Latest fingerprint" flush />
        <div className="mt-4 space-y-4">
          <HashLine label="Balance fingerprint" value={summary?.state_root} />
          <HashLine label="Latest activity fingerprint" value={summary?.latest_block_hash} />
          <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
            <div className="text-xs uppercase tracking-[0.18em] text-white/38">Selected activity flow</div>
            <div className="mt-3 flex items-center gap-2 text-sm">
              <span className="hash min-w-0 flex-1 text-white/60">{shortHash(selectedBlock?.parent_hash, 8, 8)}</span>
              <ArrowRight className="h-4 w-4 shrink-0 text-amber" />
              <span className="hash min-w-0 flex-1 text-white">{shortHash(selectedBlock?.block_hash, 8, 8)}</span>
            </div>
          </div>
          {selectedBlock?.transactions.map((tx, index) => (
            <TransactionMini key={`${tx.signature_base64}-${index}`} tx={tx} onSelectAccount={onSelectAccount} />
          ))}
        </div>
      </Panel>
    </div>
  );
}

function WorkflowStrip({
  latestBlock,
  transferCount,
  hasSettlement,
}: {
  latestBlock: number | null;
  transferCount: number;
  hasSettlement: boolean;
}) {
  const steps = [
    {
      icon: <Send className="h-4 w-4" />,
      label: "Transfers arrive",
      value: transferCount > 0 ? pluralize(transferCount, "transfer") : "none selected",
      tone: transferCount > 0 ? "good" : "neutral",
    },
    {
      icon: <Blocks className="h-4 w-4" />,
      label: "Grouped into blocks",
      value: latestBlock === null ? "waiting" : `latest ${latestBlock}`,
      tone: latestBlock === null ? "neutral" : "good",
    },
    {
      icon: <PackageCheck className="h-4 w-4" />,
      label: "Packed for settlement",
      value: hasSettlement ? "ready" : "see Settlement",
      tone: hasSettlement ? "good" : "warn",
    },
    {
      icon: <ShieldAlert className="h-4 w-4" />,
      label: "Anchored on L1",
      value: "tracked per batch",
      tone: "neutral",
    },
  ] as const;

  return (
    <Panel className="p-4">
      <div className="grid grid-cols-1 gap-3 md:grid-cols-4">
        {steps.map((step, index) => (
          <div key={step.label} className="flex min-w-0 items-center gap-3 rounded-lg border border-white/10 bg-white/[0.035] p-3">
            <div className={cn(
              "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg",
              step.tone === "good" ? "bg-mint/10 text-mint" : step.tone === "warn" ? "bg-amber/10 text-amber" : "bg-white/[0.06] text-white/58",
            )}>
              {step.icon}
            </div>
            <div className="min-w-0">
              <div className="text-xs text-white/42">Step {index + 1}</div>
              <div className="text-sm font-semibold text-white">{step.label}</div>
              <div className="mt-0.5 text-xs text-white/50">{step.value}</div>
            </div>
          </div>
        ))}
      </div>
    </Panel>
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
  transactionError,
  onSelectBlock,
  onSelectAccount,
  onSelectTx,
}: {
  blocks: BlockSummary[];
  blockTotal: number;
  selectedHeight: number | null;
  selectedBlock?: Block;
  blockLoading: boolean;
  selectedTx: string | null;
  transaction?: { block_height: number | null; transaction_index: number; transfer: Transfer; hash: string };
  transactionError: string | null;
  onSelectBlock: (height: number) => void;
  onSelectAccount: (address: string) => void;
  onSelectTx: (hash: string) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[23rem_1fr]">
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

      <div className="grid gap-4">
        <Panel className="p-4">
          <SectionTitle icon={<Database className="h-4 w-4" />} title="Activity group detail" action={selectedBlock ? `block ${selectedBlock.height}` : "none"} flush />
          {blockLoading ? (
            <Skeleton className="mt-4 h-56" />
          ) : selectedBlock ? (
            <div className="mt-4 grid gap-3">
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                <HashLine label="Group fingerprint" value={selectedBlock.block_hash} />
                <HashLine label="Previous group fingerprint" value={selectedBlock.parent_hash} />
                <HashLine label="Balance fingerprint" value={selectedBlock.state_root} />
                <MetricLine label="Transfers" value={selectedBlock.transactions.length} />
              </div>
              <div className="overflow-hidden rounded-lg border border-white/10">
                <div className="grid grid-cols-[1fr_auto_1fr_auto] gap-3 border-b border-white/10 bg-white/[0.045] px-3 py-2 text-xs uppercase tracking-[0.14em] text-white/38">
                  <span>Sender</span>
                  <span>Amount</span>
                  <span>Receiver</span>
                  <span>Open</span>
                </div>
                {selectedBlock.transactions.length === 0 ? (
                  <div className="px-3 py-6 text-sm text-white/48">The first block only starts the system, so it has no transfers.</div>
                ) : (
                  selectedBlock.transactions.map((tx, index) => (
                    <TransactionRow
                      key={`${tx.signature_base64}-${index}`}
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

        <Panel className="p-4">
          <SectionTitle icon={<Activity className="h-4 w-4" />} title="Transfer lookup" action={selectedTx ? shortHash(selectedTx) : "paste fingerprint"} flush />
          {transaction ? (
            <div className="mt-4 grid gap-3 md:grid-cols-2">
              <HashLine label="Transfer fingerprint" value={transaction.hash} />
              <MetricLine label="Activity group" value={transaction.block_height ?? "waiting"} />
              <TransferDetail tx={transaction.transfer} onSelectAccount={onSelectAccount} />
            </div>
          ) : transactionError ? (
            <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{transactionError}</div>
          ) : (
            <div className="mt-4 text-sm text-white/50">Search a transfer fingerprint or open one from an activity row.</div>
          )}
        </Panel>
      </div>
    </div>
  );
}

function AccountsView({
  candidates,
  selectedAccount,
  account,
  accountError,
  onSelectAccount,
}: {
  candidates: Array<{ address: string; touches: number; sent: number; received: number }>;
  selectedAccount: string | null;
  account?: Account;
  accountError: string | null;
  onSelectAccount: (address: string) => void;
}) {
  const [manual, setManual] = useState("");

  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[25rem_1fr]">
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

      <Panel className="p-4">
        <SectionTitle icon={<Wallet className="h-4 w-4" />} title="Wallet detail" action={selectedAccount ? shortHash(selectedAccount) : "none"} flush />
        {account ? (
          <div className="mt-4 grid gap-3 md:grid-cols-3">
            <HashLine label="Wallet address" value={account.address} />
            <MetricLine label="Balance" value={account.balance} />
            <MetricLine label="Next transfer #" value={account.nonce} />
          </div>
        ) : accountError ? (
          <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{accountError}</div>
        ) : (
          <div className="mt-4 text-sm text-white/50">Select a wallet from the current activity group or paste an address.</div>
        )}
      </Panel>
    </div>
  );
}

function SequencerView({
  pool,
  poolTotal,
  onSubmitted,
  onSequenced,
}: {
  pool: TransactionPoolEntry[];
  poolTotal: number;
  onSubmitted: (hash: string) => Promise<void>;
  onSequenced: (height: number) => Promise<void>;
}) {
  const [transferJson, setTransferJson] = useState("");
  const [limit, setLimit] = useState(100);
  const [submitting, setSubmitting] = useState(false);
  const [sequencing, setSequencing] = useState(false);
  const [notice, setNotice] = useState<{ tone: "good" | "bad" | "neutral"; message: string } | null>(null);

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
        message: `Built activity group ${result.block.height} with ${pluralize(result.included_transactions.length, "transfer")}.`,
      });
      await onSequenced(result.block.height);
    } catch (error) {
      setNotice({ tone: "bad", message: error instanceof Error ? error.message : "Batch sequencing failed" });
    } finally {
      setSequencing(false);
    }
  }

  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[0.85fr_1.15fr]">
      <div className="grid gap-4">
        <Panel className="p-4">
          <SectionTitle icon={<Send className="h-4 w-4" />} title="Add a signed transfer" action="JSON from wallet CLI" flush />
          <textarea
            value={transferJson}
            onChange={(event) => setTransferJson(event.target.value)}
            placeholder='{"from":"...","to":"...","amount":5,"nonce":0,"public_key_base64":"...","signature_base64":"..."}'
            className="mt-4 min-h-44 w-full resize-y rounded-lg border border-white/10 bg-black/20 p-3 font-mono text-sm leading-6 text-white outline-none placeholder:text-white/28 focus:border-mint/40"
          />
          <button
            onClick={submitTransfer}
            disabled={submitting || transferJson.trim().length === 0}
            className="mt-3 inline-flex items-center gap-2 rounded-lg bg-mint px-3 py-2 text-sm font-semibold text-ink transition hover:bg-mint/90 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Send className="h-4 w-4" />
            Add transfer
          </button>
        </Panel>

        <Panel className="p-4">
          <SectionTitle icon={<Play className="h-4 w-4" />} title="Group waiting transfers" action="Local builder" flush />
          <div className="mt-4 grid gap-3 sm:grid-cols-[1fr_auto]">
            <label className="grid gap-2 text-sm text-white/62">
              Max transfers
              <input
                type="number"
                min={1}
                max={100}
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
                className="h-10 rounded-lg border border-white/10 bg-white/[0.06] px-3 text-white outline-none focus:border-mint/40"
              />
            </label>
            <button
              onClick={sequenceBatch}
              disabled={sequencing}
              className="self-end inline-flex h-10 items-center justify-center gap-2 rounded-lg bg-cyan px-3 text-sm font-semibold text-ink transition hover:bg-cyan/90 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Play className="h-4 w-4" />
              Build group
            </button>
          </div>
        </Panel>

        {notice && (
          <Panel className={cn("p-3", notice.tone === "bad" ? "border-rose/25 bg-rose/10" : "border-mint/25 bg-mint/10")}>
            <div className={cn("text-sm font-medium", notice.tone === "bad" ? "text-rose" : "text-mint")}>{notice.message}</div>
          </Panel>
        )}
      </div>

      <Panel className="overflow-hidden">
        <SectionTitle icon={<Activity className="h-4 w-4" />} title="Waiting transfers" action={`${pool.length}/${poolTotal} loaded`} />
        <div className="max-h-[42rem] overflow-auto">
          {pool.length === 0 ? (
            <div className="px-4 py-10 text-sm text-white/48">No transfers are waiting to be grouped.</div>
          ) : (
            pool.map((entry) => (
              <div key={entry.id} className="grid gap-3 border-b border-white/8 px-4 py-3 last:border-b-0">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="hash min-w-0 text-sm text-white">{entry.hash}</div>
                  <Badge tone={entry.status === "included" ? "good" : entry.status === "failed" ? "bad" : "warn"}>{entry.status}</Badge>
                </div>
                <div className="grid gap-2 text-xs text-white/48 sm:grid-cols-3">
                  <span className="hash">sender {shortHash(entry.transfer.from, 8, 6)}</span>
                  <span className="hash">receiver {shortHash(entry.transfer.to, 8, 6)}</span>
                  <span>amount {entry.transfer.amount} / transfer #{entry.transfer.nonce}</span>
                </div>
                {entry.included_block_height !== null && (
                  <div className="text-xs text-mint">
                    grouped in block {entry.included_block_height} at position {entry.included_transaction_index}
                  </div>
                )}
                {entry.failure_reason && <div className="text-xs text-rose">{entry.failure_reason}</div>}
              </div>
            ))
          )}
        </div>
      </Panel>
    </div>
  );
}

function HealthView({
  health,
  errors,
  summary,
}: {
  health?: { status: string; latest_error: unknown };
  errors: Array<{ id: number; severity: string; message: string; service: string; created_at: string; resolved_at: string | null }>;
  summary?: { height: number | null; valid: boolean; block_count: number };
}) {
  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[0.85fr_1.15fr]">
      <Panel className="p-4">
        <SectionTitle icon={<ShieldAlert className="h-4 w-4" />} title="System status" action={health?.status ?? "optional"} flush />
        <div className="mt-4 grid gap-3">
          <StatusLine label="API connection" ok={health?.status === "ok"} value={health?.status ?? "unavailable"} />
          <StatusLine label="Data consistency" ok={Boolean(summary?.valid)} value={summary?.valid ? "healthy" : "needs review"} />
          <StatusLine label="Latest activity group" ok value={summary?.height ?? "none"} />
          <StatusLine label="Stored groups" ok value={summary?.block_count ?? 0} />
        </div>
      </Panel>
      <Panel className="overflow-hidden">
        <SectionTitle icon={<Activity className="h-4 w-4" />} title="System events" action={`${errors.length} recent`} />
        {errors.length === 0 ? (
          <div className="px-4 py-10 text-sm text-white/48">No system issues are currently exposed.</div>
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
    </div>
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
                  <Badge tone={settlementStatusTone(batch.status)}>{batch.status}</Badge>
                </div>
                <div className="mt-2 text-xs text-white/46">
                  groups {batch.start_block_height}-{batch.end_block_height} / {pluralize(batch.transaction_count, "transfer")}
                </div>
                {batch.l1_submit_tx_hash && (
                  <div className="mt-2 text-xs text-mint/72">L1 {shortHash(batch.l1_submit_tx_hash, 10, 8)}</div>
                )}
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
            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
              <StatusLine label="Settlement state" ok={selectedBatch.status === "finalized"} value={<Badge tone={settlementStatusTone(selectedBatch.status)}>{settlementStatusLabel(selectedBatch.status)}</Badge>} />
              <MetricLine label="Settlement chain" value={selectedBatch.l1_chain_id ?? "none"} />
              <MetricLine label="Sent to settlement" value={selectedBatch.l1_submitted_at ? "yes" : "no"} />
            </div>
            <div className="grid grid-cols-1 gap-3 md:grid-cols-4">
              <MetricLine label="Send block" value={selectedBatch.l1_submit_block_number ?? "none"} />
              <MetricLine label="Send gas" value={selectedBatch.l1_submit_gas_used ?? "none"} />
              <MetricLine label="Final block" value={selectedBatch.l1_finalize_block_number ?? "none"} />
              <MetricLine label="Final gas" value={selectedBatch.l1_finalize_gas_used ?? "none"} />
            </div>
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <HashLine label="Previous balance fingerprint" value={selectedBatch.prev_state_root} />
              <HashLine label="New balance fingerprint" value={selectedBatch.new_state_root} />
              <HashLine label="Transfer fingerprint" value={selectedBatch.transaction_root} />
              <HashLine label="Group fingerprint" value={selectedBatch.block_hash_root} />
              <HashLine label="Package fingerprint" value={selectedBatch.data_hash} />
            </div>
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <HashLine label="Settlement contract" value={selectedBatch.l1_contract_address} />
              <HashLine label="Send receipt" value={selectedBatch.l1_submit_tx_hash} />
              <HashLine label="Final receipt" value={selectedBatch.l1_finalize_tx_hash} />
              <HashLine label="Finalized at" value={selectedBatch.l1_finalized_at} />
            </div>
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
        {value && <CopyButton value={value} />}
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

function MetricLine({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="mb-2 text-xs uppercase tracking-[0.16em] text-white/36">{label}</div>
      <div className="break-words text-xl font-semibold leading-tight text-white sm:text-2xl">{value}</div>
    </div>
  );
}

function StatusLine({ label, ok, value }: { label: string; ok: boolean; value: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="flex items-center gap-2 text-sm text-white/68">
        {ok ? <CheckCircle2 className="h-4 w-4 text-mint" /> : <XCircle className="h-4 w-4 text-rose" />}
        {label}
      </div>
      <div className="text-sm font-medium text-white">{value}</div>
    </div>
  );
}

function CopyButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      title="Copy"
      aria-label="Copy"
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

function TransactionMini({ tx, onSelectAccount }: { tx: Transfer; onSelectAccount: (address: string) => void }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="flex items-center gap-2 text-sm">
        <button onClick={() => onSelectAccount(tx.from)} className="hash min-w-0 flex-1 text-cyan hover:text-cyan/80">
          {shortHash(tx.from, 9, 6)}
        </button>
        <span className="rounded-md bg-amber/10 px-2 py-1 text-xs font-semibold text-amber">{tx.amount}</span>
        <button onClick={() => onSelectAccount(tx.to)} className="hash min-w-0 flex-1 text-mint hover:text-mint/80">
          {shortHash(tx.to, 9, 6)}
        </button>
      </div>
    </div>
  );
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
    <div className="grid grid-cols-[1fr_auto_1fr_auto] items-center gap-3 border-b border-white/8 px-3 py-3 text-sm last:border-b-0">
      <button onClick={() => onSelectAccount(tx.from)} className="hash min-w-0 text-left text-cyan hover:text-cyan/80">
        {shortHash(tx.from)}
      </button>
      <span className="rounded-md bg-amber/10 px-2 py-1 text-xs font-semibold text-amber">{tx.amount}</span>
      <button onClick={() => onSelectAccount(tx.to)} className="hash min-w-0 text-left text-mint hover:text-mint/80">
        {shortHash(tx.to)}
      </button>
      <button
        disabled={!hash}
        onClick={() => hash && onSelectTx(hash)}
        className="inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.045] px-2 py-1 text-xs text-white/58 transition hover:border-mint/30 hover:text-mint disabled:opacity-40"
      >
        <ExternalLink className="h-3 w-3" />
        {hash ? shortHash(hash, 6, 4) : "no hash"}
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
