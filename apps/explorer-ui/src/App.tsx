import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  ArrowRight,
  Blocks,
  CheckCircle2,
  Copy,
  Database,
  ExternalLink,
  Gauge,
  Hash,
  Layers3,
  RefreshCcw,
  Search,
  ShieldAlert,
  Wallet,
  XCircle,
} from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import { API_BASE, api, type Account, type Block, type BlockSummary, type SearchResult, type Transfer } from "./lib/api";
import { cn, formatNumber, pluralize, shortHash } from "./lib/utils";
import { Badge, IconButton, Panel, Shell, Skeleton, Stat } from "./components/ui";

const tabs = ["Overview", "Blocks", "Accounts", "Indexer"] as const;
type Tab = (typeof tabs)[number];

export function App() {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<Tab>("Overview");
  const [selectedHeight, setSelectedHeight] = useState<number | null>(null);
  const [selectedAccount, setSelectedAccount] = useState<string | null>(null);
  const [selectedTx, setSelectedTx] = useState<string | null>(null);
  const [searchResult, setSearchResult] = useState<SearchResult | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);

  const summary = useQuery({ queryKey: ["summary"], queryFn: api.summary });
  const blocks = useQuery({ queryKey: ["blocks"], queryFn: api.blocks });
  const health = useQuery({ queryKey: ["health"], queryFn: api.health });
  const errors = useQuery({ queryKey: ["errors"], queryFn: api.errors });
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
  const connectionError = firstError(summary.error, blocks.error);
  const connectionMessage = connectionError ?? "Backend request failed";
  const hasCoreData = Boolean(summary.data && blocks.data);
  const isOffline = Boolean(connectionError && !hasCoreData);
  const isStale = Boolean(connectionError && hasCoreData);
  const readStatus = isOffline ? "offline" : isStale ? "stale" : hasCoreData ? "ok" : "connecting";

  useEffect(() => {
    if (selectedHeight === null && blocks.data?.[0]) {
      setSelectedHeight(blocks.data[0].height);
    }
  }, [blocks.data, selectedHeight]);

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
      <header className="flex flex-col gap-4 border-b border-white/10 pb-4 lg:flex-row lg:items-center lg:justify-between">
        <div className="flex items-center gap-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-lg border border-mint/25 bg-mint/10 text-mint shadow-glow">
            <Layers3 className="h-5 w-5" />
          </div>
          <div>
            <h1 className="text-2xl font-semibold leading-tight text-white sm:text-3xl">RollupLens Explorer</h1>
            <div className="mt-1 flex flex-wrap items-center gap-2 text-sm text-white/56">
              <Badge tone={readStatus === "ok" ? "good" : "warn"}>
                {readStatus}
              </Badge>
              <span>Live chain read path</span>
              <span className="hidden text-white/24 sm:inline">/</span>
              <span className="hash max-w-[15rem]">{shortHash(summary.data?.latest_block_hash)}</span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <SearchBox onSearch={onSearch} pending={false} />
          <IconButton onClick={refreshAll} title="Refresh data" aria-label="Refresh data">
            <RefreshCcw className="h-4 w-4" />
          </IconButton>
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

      <nav className="flex gap-2 overflow-x-auto pb-1">
        {tabs.map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={cn(
              "relative rounded-lg px-4 py-2 text-sm font-medium text-white/58 transition hover:text-white",
              activeTab === tab && "text-white",
            )}
          >
            {activeTab === tab && (
              <motion.span
                layoutId="active-tab"
                className="absolute inset-0 rounded-lg border border-mint/25 bg-mint/10"
                transition={{ type: "spring", bounce: 0.2, duration: 0.5 }}
              />
            )}
            <span className="relative">{tab}</span>
          </button>
        ))}
      </nav>

      <section className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-4">
        {summary.isLoading ? (
          <>
            <Skeleton className="h-32" />
            <Skeleton className="h-32" />
            <Skeleton className="h-32" />
            <Skeleton className="h-32" />
          </>
        ) : (
          <>
            <Stat label="Height" value={summary.data?.height ?? 0} detail="Latest finalized local block" accent="mint" />
            <Stat label="Transactions" value={formatNumber(summary.data?.transaction_count)} detail={`${formatNumber(summary.data?.pending_transaction_count)} pending in mempool`} accent="cyan" />
            <Stat label="Accounts" value={formatNumber(summary.data?.account_count)} detail="Current account snapshots" accent="amber" />
            <Stat label="Validity" value={summary.data?.valid ? "Valid" : "Invalid"} detail={summary.data?.genesis_finalized ? "Genesis finalized" : "Genesis not finalized"} accent={summary.data?.valid ? "mint" : "rose"} />
          </>
        )}
      </section>

      {activeTab === "Overview" && (
        <Overview
          summary={summary.data}
          blocks={blocks.data ?? []}
          selectedBlock={block.data}
          onSelectBlock={setSelectedHeight}
          onSelectAccount={(address) => {
            setSelectedAccount(address);
            setActiveTab("Accounts");
          }}
        />
      )}

      {activeTab === "Blocks" && (
        <BlocksView
          blocks={blocks.data ?? []}
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
      )}

      {activeTab === "Accounts" && (
        <AccountsView
          candidates={accountsFromBlocks}
          selectedAccount={selectedAccount}
          account={account.data}
          accountError={account.error instanceof Error ? account.error.message : null}
          onSelectAccount={setSelectedAccount}
        />
      )}

      {activeTab === "Indexer" && (
        <IndexerView health={health.data} errors={errors.data ?? []} summary={summary.data} />
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

function Overview({
  summary,
  blocks,
  selectedBlock,
  onSelectBlock,
  onSelectAccount,
}: {
  summary?: { state_root: string; latest_block_hash: string | null; block_count: number };
  blocks: BlockSummary[];
  selectedBlock?: Block;
  onSelectBlock: (height: number) => void;
  onSelectAccount: (address: string) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-[1.2fr_0.8fr]">
      <Panel className="overflow-hidden">
        <SectionTitle icon={<Blocks className="h-4 w-4" />} title="Latest blocks" action={pluralize(blocks.length, "block")} />
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
                <div className="mt-1 text-xs text-white/45">{pluralize(block.transaction_count, "transaction")}</div>
              </div>
              <ArrowRight className="h-4 w-4 text-white/34" />
            </button>
          ))}
        </div>
      </Panel>

      <Panel className="p-4">
        <SectionTitle icon={<Gauge className="h-4 w-4" />} title="State" action="Current root" flush />
        <div className="mt-4 space-y-4">
          <HashLine label="State root" value={summary?.state_root} />
          <HashLine label="Latest block" value={summary?.latest_block_hash} />
          <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
            <div className="text-xs uppercase tracking-[0.18em] text-white/38">Selected block flow</div>
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

function BlocksView({
  blocks,
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
        <SectionTitle icon={<Blocks className="h-4 w-4" />} title="Block stream" action={`${blocks.length} indexed`} />
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
                <div className="mt-1 text-xs text-white/42">{pluralize(block.transaction_count, "tx", "txs")}</div>
              </div>
            </button>
          ))}
        </div>
      </Panel>

      <div className="grid gap-4">
        <Panel className="p-4">
          <SectionTitle icon={<Database className="h-4 w-4" />} title="Block detail" action={selectedBlock ? `height ${selectedBlock.height}` : "none"} flush />
          {blockLoading ? (
            <Skeleton className="mt-4 h-56" />
          ) : selectedBlock ? (
            <div className="mt-4 grid gap-3">
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                <HashLine label="Block hash" value={selectedBlock.block_hash} />
                <HashLine label="Parent hash" value={selectedBlock.parent_hash} />
                <HashLine label="State root" value={selectedBlock.state_root} />
                <MetricLine label="Transactions" value={selectedBlock.transactions.length} />
              </div>
              <div className="overflow-hidden rounded-lg border border-white/10">
                <div className="grid grid-cols-[1fr_auto_1fr_auto] gap-3 border-b border-white/10 bg-white/[0.045] px-3 py-2 text-xs uppercase tracking-[0.14em] text-white/38">
                  <span>From</span>
                  <span>Amount</span>
                  <span>To</span>
                  <span>Hash</span>
                </div>
                {selectedBlock.transactions.length === 0 ? (
                  <div className="px-3 py-6 text-sm text-white/48">Genesis block has no transactions.</div>
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
            <div className="mt-4 text-sm text-white/50">Select a block from the stream.</div>
          )}
        </Panel>

        <Panel className="p-4">
          <SectionTitle icon={<Activity className="h-4 w-4" />} title="Transaction lookup" action={selectedTx ? shortHash(selectedTx) : "paste hash"} flush />
          {transaction ? (
            <div className="mt-4 grid gap-3 md:grid-cols-2">
              <HashLine label="Transaction hash" value={transaction.hash} />
              <MetricLine label="Block height" value={transaction.block_height ?? "pending"} />
              <TransferDetail tx={transaction.transfer} onSelectAccount={onSelectAccount} />
            </div>
          ) : transactionError ? (
            <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{transactionError}</div>
          ) : (
            <div className="mt-4 text-sm text-white/50">Search a transaction hash or use a computed hash from a block row.</div>
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
        <SectionTitle icon={<Wallet className="h-4 w-4" />} title="Account lookup" action="Current state" flush />
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
            placeholder="Paste account address"
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
                <span>{pluralize(candidate.touches, "touch", "touches")}</span>
                <span>sent {candidate.sent}</span>
                <span>received {candidate.received}</span>
              </div>
            </button>
          ))}
        </div>
      </Panel>

      <Panel className="p-4">
        <SectionTitle icon={<Wallet className="h-4 w-4" />} title="Account detail" action={selectedAccount ? shortHash(selectedAccount) : "none"} flush />
        {account ? (
          <div className="mt-4 grid gap-3 md:grid-cols-3">
            <HashLine label="Address" value={account.address} />
            <MetricLine label="Balance" value={account.balance} />
            <MetricLine label="Nonce" value={account.nonce} />
          </div>
        ) : accountError ? (
          <div className="mt-4 rounded-lg border border-rose/20 bg-rose/10 p-3 text-sm text-rose">{accountError}</div>
        ) : (
          <div className="mt-4 text-sm text-white/50">Select an account touched by the current block or paste an address.</div>
        )}
      </Panel>
    </div>
  );
}

function IndexerView({
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
        <SectionTitle icon={<ShieldAlert className="h-4 w-4" />} title="Read path health" action={health?.status ?? "optional"} flush />
        <div className="mt-4 grid gap-3">
          <StatusLine label="Health endpoint" ok={health?.status === "ok"} value={health?.status ?? "unavailable"} />
          <StatusLine label="Chain validation" ok={Boolean(summary?.valid)} value={summary?.valid ? "valid" : "invalid"} />
          <StatusLine label="Indexed height" ok value={summary?.height ?? "none"} />
          <StatusLine label="Block rows" ok value={summary?.block_count ?? 0} />
        </div>
      </Panel>
      <Panel className="overflow-hidden">
        <SectionTitle icon={<Activity className="h-4 w-4" />} title="Error events" action={`${errors.length} recent`} />
        {errors.length === 0 ? (
          <div className="px-4 py-10 text-sm text-white/48">No persistent indexer errors are currently exposed.</div>
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

function MetricLine({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/10 bg-white/[0.04] p-3">
      <div className="mb-2 text-xs uppercase tracking-[0.16em] text-white/36">{label}</div>
      <div className="text-2xl font-semibold text-white">{value}</div>
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
      <HashLine label="From" value={tx.from} />
      <HashLine label="To" value={tx.to} />
      <MetricLine label="Amount" value={tx.amount} />
      <MetricLine label="Nonce" value={tx.nonce} />
      <button onClick={() => onSelectAccount(tx.from)} className="rounded-lg border border-cyan/20 bg-cyan/10 p-3 text-left text-sm font-medium text-cyan">
        Open sender
      </button>
      <button onClick={() => onSelectAccount(tx.to)} className="rounded-lg border border-mint/20 bg-mint/10 p-3 text-left text-sm font-medium text-mint">
        Open receiver
      </button>
    </div>
  );
}
