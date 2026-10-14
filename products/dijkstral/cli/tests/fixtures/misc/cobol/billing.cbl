      * Billing batch
       IDENTIFICATION DIVISION.
       PROGRAM-ID. BILLING.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-TOTAL PIC 9(5).
       01 WS-REC.
          05 WS-NAME PIC X(10).
       PROCEDURE DIVISION.
       MAIN-PARA.
           PERFORM INIT-PARA
           PERFORM CALC-PARA THRU CALC-EXIT
           PERFORM MISSING-PARA
           STOP RUN.
       INIT-PARA.
           MOVE 0 TO WS-TOTAL.
       CALC-SECTION SECTION.
       CALC-PARA.
           ADD 1 TO WS-TOTAL.
       CALC-EXIT.
           EXIT.
