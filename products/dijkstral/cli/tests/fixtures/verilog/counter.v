module counter(input clk, output reg q);
  always @(posedge clk) q <= ~q;
endmodule
