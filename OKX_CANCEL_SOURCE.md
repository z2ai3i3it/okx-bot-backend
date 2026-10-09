Source of the order cancellation.\
Valid values and the corresponding meanings are:\
`0`: Order canceled by system\
`1`: Order canceled by user\
`2`: Order canceled: Pre reduce-only order canceled, due to insufficient margin in user position\
`3`: Order canceled: Risk cancellation was triggered. Pending order was canceled due to insufficient maintenance margin ratio and forced-liquidation risk.\
`4`: Order canceled: Borrowings of crypto reached hard cap, order was canceled by system.\
`6`: Order canceled: ADL order cancellation was triggered. Pending order was canceled due to a low margin ratio and forced-liquidation risk.\
`7`: Order canceled: Futures contract delivery.\
`9`: Order canceled: Insufficient balance after funding fees deducted.\
`10`: Order canceled: Option contract expiration.\
`13`: Order canceled: FOK order was canceled due to incompletely filled.\
`14`: Order canceled: IOC order was partially canceled due to incompletely filled.\
`15`: Order canceled: The order price is beyond the limit\
`17`: Order canceled: Close order was canceled, due to the position was already closed at market price.\
`20`: Cancel all after triggered\
`21`: Order canceled: The TP/SL order was canceled because the position had been closed\
`22` Order canceled: Due to a better price was available for the order in the same direction, the current operation reduce-only order was automatically canceled\
`23` Order canceled: Due to a better price was available for the order in the same direction, the existing reduce-only order was automatically canceled\
`27`: Order canceled: Price limit verification failed because the price difference between counterparties exceeds 5%\
`31`: The post-only order will take liquidity in taker orders\
`32`: Self trade prevention\
`33`: The order exceeds the maximum number of order matches per taker order\
`36`: Your TP limit order was canceled because the corresponding SL order was triggered.\
`37`: Your TP limit order was canceled because the corresponding SL order was canceled.\
`38`: You have canceled market maker protection (MMP) orders.\
`39`: Your order was canceled because market maker protection (MMP) was triggered.\
`42`: Your order was canceled because the difference between the initial and current best bid or ask prices reached the maximum chase difference.\
`43`: Order cancelled because the buy order price is higher than the index price or the sell order price is lower than the index price.\
`44`: Your order was canceled because your available balance of this crypto was insufficient for auto conversion. Auto conversion was triggered when the total collateralized liabilities for this crypto reached the platform’s risk control limit.\
`45`: Order cancelled because RPI order price verification failed\
`46`: delta reducing cancel orders

ใช่ครับ รายการที่คุณส่งมาคือรายละเอียด `cancelSource` ที่ควรใช้เป็นฐานในการทำ Mapping สำหรับ OKX Order WebSocket event และมีค่าเพิ่มเติมหลายค่าที่ผมไม่ได้ระบุไว้ก่อนหน้า

สำหรับระบบของคุณ ผมแนะนำให้จัดกลุ่มดังนี้

| กลุ่ม                                | ค่า `cancelSource`                                   | การจัดการ                                          |
| ------------------------------------ | ---------------------------------------------------- | -------------------------------------------------- |
| System / Unknown source              | `0`                                                  | Unplanned เว้นแต่มีเหตุผลที่ระบบรู้จัก             |
| User cancel                          | `1`                                                  | ตรวจสอบว่าเป็น planned cancel หรือ external cancel |
| Risk / Margin / ADL                  | `2`, `3`, `6`, `9`, `44`                             | Unplanned และตรวจสอบสถานะบัญชี                     |
| Expiration / Delivery                | `7`, `10`                                            | จัดการตาม lifecycle ของ contract                   |
| Execution policy / Order constraints | `13`, `14`, `15`, `27`, `31`, `32`, `33`, `43`, `45` | แยกตามเหตุผล ไม่ควรมองว่าเป็นการยกเลิกโดยผู้ใช้    |
| TP/SL / Position state               | `17`, `21`, `36`, `37`                               | ตรวจสอบสถานะ position และคำสั่งที่เกี่ยวข้อง       |
| Automatic order management           | `20`, `22`, `23`, `38`, `39`, `42`, `46`             | ตรวจสอบเงื่อนไขเฉพาะของแต่ละประเภทคำสั่ง           |
| Borrowing hard cap                   | `4`                                                  | Unplanned และตรวจสอบความเสี่ยงด้านการกู้ยืม        |

ข้อควรระวัง: การจัดกลุ่มนี้เป็นข้อเสนอสำหรับการออกแบบระบบ ไม่ใช่การรับรองว่า OKX จะส่งทุกค่าในทุกประเภทบัญชีหรือทุกตลาด

### กฎที่ควรใช้จริง

ผมจะไม่ใช้กฎ `cancelSource != "1" → Unplanned` แบบตรง ๆ เพียงอย่างเดียว แต่จะใช้ planned-cancel tracking เป็นตัวตัดสินหลัก แล้วใช้ `cancelSource` เพื่อระบุสาเหตุและเลือกการตอบสนอง

```
if is_planned_cancel(order.ordId):    handle_planned_cancel(order)else:    handle_unplanned_cancel(        order,        cancel_source=order.cancelSource,    )
```

วิธีนี้ช่วยให้ระบบแยกได้ว่า order ถูกยกเลิกเพราะระบบของเราสั่งเอง หรือเกิดจาก OKX, ผู้ใช้ภายนอก หรือกลไกจัดการคำสั่งอื่น โดยไม่ต้องพึ่ง `cancelSource` เพียงอย่างเดียว