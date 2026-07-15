import type { EnterpriseBudgetEvent } from '@/types'

export interface BudgetEventAmountLine {
  dimension: 'reserved' | 'settled' | 'none'
  label: string
  microunits: number
}

/**
 * Budget events update two independent ledger dimensions. Keep them separate in
 * the UI: adding the deltas can make a successful settlement look like a credit
 * whenever the final charge is lower than the original reservation.
 */
export function budgetEventAmountLines(
  event: Pick<EnterpriseBudgetEvent, 'reservedDeltaMicrounits' | 'settledDeltaMicrounits'>,
): BudgetEventAmountLine[] {
  const amounts: BudgetEventAmountLine[] = []

  if (event.reservedDeltaMicrounits !== 0) {
    amounts.push({
      dimension: 'reserved',
      label: '预留变动',
      microunits: event.reservedDeltaMicrounits,
    })
  }
  if (event.settledDeltaMicrounits !== 0) {
    amounts.push({
      dimension: 'settled',
      label: '结算变动',
      microunits: event.settledDeltaMicrounits,
    })
  }

  return amounts.length > 0
    ? amounts
    : [{ dimension: 'none', label: '账务变动', microunits: 0 }]
}
