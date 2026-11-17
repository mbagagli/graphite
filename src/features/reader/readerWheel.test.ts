import { expect, it } from "vitest";
import { ReaderWheelGate } from "./readerWheel";

it("turns only once during a trackpad gesture and rearms after a pause", () => {
  const gate = new ReaderWheelGate();
  expect(gate.shouldTurn(50, true, 0)).toBe(false);
  expect(gate.shouldTurn(50, true, 20)).toBe(true);
  for (let time = 40; time <= 400; time += 20) {
    expect(gate.shouldTurn(50, true, time)).toBe(false);
  }
  expect(gate.shouldTurn(50, true, 700)).toBe(false);
  expect(gate.shouldTurn(50, true, 720)).toBe(true);
});

it("requires a consistent direction at the boundary", () => {
  const gate = new ReaderWheelGate();
  expect(gate.shouldTurn(60, false, 0)).toBe(false);
  expect(gate.shouldTurn(60, true, 20)).toBe(false);
  expect(gate.shouldTurn(-60, true, 40)).toBe(false);
  expect(gate.shouldTurn(-60, true, 60)).toBe(true);
  gate.block();
  expect(gate.shouldTurn(60, true, 80)).toBe(false);
  expect(gate.shouldTurn(NaN, true, 300)).toBe(false);
});
