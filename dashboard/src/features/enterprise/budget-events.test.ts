import { describe, expect, it } from 'vitest'
import { budgetEventAmountLines } from './budget-events'

describe('budget event amount lines', () => {
  it('keeps reservation release and final settlement as separate ledger dimensions', () => {
    const amounts = budgetEventAmountLines({
      reservedDeltaMicrounits: -750_000,
      settledDeltaMicrounits: 625_123,
    })

    expect(amounts).toEqual([
      { dimension: 'reserved', label: '预留变动', microunits: -750_000 },
      { dimension: 'settled', label: '结算变动', microunits: 625_123 },
    ])
    expect(amounts.map((amount) => amount.microunits)).not.toContain(-124_877)
  })

  it('shows a single dimension for reservation and adjustment events', () => {
    expect(budgetEventAmountLines({
      reservedDeltaMicrounits: 1_000_000,
      settledDeltaMicrounits: 0,
    })).toEqual([
      { dimension: 'reserved', label: '预留变动', microunits: 1_000_000 },
    ])

    expect(budgetEventAmountLines({
      reservedDeltaMicrounits: 0,
      settledDeltaMicrounits: -250_000,
    })).toEqual([
      { dimension: 'settled', label: '结算变动', microunits: -250_000 },
    ])
  })

  it('keeps zero-value evidence visible', () => {
    expect(budgetEventAmountLines({
      reservedDeltaMicrounits: 0,
      settledDeltaMicrounits: 0,
    })).toEqual([
      { dimension: 'none', label: '账务变动', microunits: 0 },
    ])
  })
})
