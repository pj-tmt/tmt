//! Values read from one held usage report: the tokens a seat group observed in the shared
//! TOK range and what its input tokens cost at the list price. Every surface that shows
//! either (the Digest tab and the Agents list) asks here, so they never disagree.
use super::Report;
use crate::board::app::App;

pub(in crate::board) fn tokens(report: Option<&Report>, ids: &[&str]) -> Option<(u128, bool)> {
    let report = report?;
    let mut total = 0;
    let mut known = 0;
    let mut partial = false;
    let mut window = None;
    for id in ids {
        if let Some((seat, tokens)) = report
            .seat(id)
            .and_then(|seat| Some((seat, seat.observed_total()?)))
        {
            let source_window = (seat.from_ms, seat.to_ms);
            if window.is_some_and(|old| old != source_window) {
                return None;
            }
            window = Some(source_window);
            total += u128::from(tokens);
            known += 1;
            partial |= !seat.complete;
        } else {
            partial = true;
        }
    }
    (known > 0).then_some((total, partial))
}
pub(in crate::board) fn input_price(
    app: &App,
    report: Option<&Report>,
    ids: &[&str],
) -> Option<u64> {
    let report = report?;
    // A price total must cover the same admitted source window as TOK. Partial
    // or absent usage cannot acquire a precise-looking price through attribution.
    let (_, partial) = tokens(Some(report), ids)?;
    if partial {
        return None;
    }
    let (status, false) = app.labels.digest()? else {
        return None;
    };
    let prices = &status.pricing.as_ref()?.models;
    let mut millionths = 0u128;
    if ids.is_empty() {
        return None;
    }
    for id in ids {
        for (model, input) in report.seat(id)?.model_inputs.as_ref()? {
            let rate = prices
                .iter()
                .find(|price| &price.model_id == model)?
                .usd_micros_per_million_input_tokens;
            millionths = millionths.checked_add(u128::from(*input) * u128::from(rate))?;
        }
    }
    u64::try_from((millionths + 500_000) / 1_000_000).ok()
}
